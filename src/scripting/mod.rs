//! Bounded Rhai extensions exchanging only maps, arrays and integer geometry.
//!
//! Layout functions receive one context map and return placement maps containing
//! `window`, `x`, `y`, `width`, `height`. Action functions take no arguments and
//! return maps using the same `action` tags as configuration bindings.
//! Top-level statements are never evaluated; extensions must be self-contained
//! functions. Errors are logged and returned so the platform can use a built-in
//! layout or retain its last good host. No actions are executed by this module.

use std::{
    collections::BTreeSet,
    fs::File,
    io::Read,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use rhai::{
    AST, Array, CallFnOptions, Dynamic, Engine, INT, Map, Scope,
    module_resolvers::DummyModuleResolver,
};

use crate::input::{Action, valid_function_name};

const MAX_SOURCE_BYTES: usize = 256 * 1024;
const MAX_WINDOWS: usize = 1024;
const MAX_ACTIONS: usize = 128;
const MAX_COORDINATE: i64 = 1_000_000;

/// Integer logical-pixel rectangle passed across the scripting boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScriptRect {
    /// Left edge in compositor coordinates.
    pub x: i32,
    /// Top edge in compositor coordinates.
    pub y: i32,
    /// Width in logical pixels; layout areas and placements must be positive.
    pub width: i32,
    /// Height in logical pixels; layout areas and placements must be positive.
    pub height: i32,
}

/// A window's stable identity and current rectangle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptWindow {
    /// Stable platform-owned window ID.
    pub id: u64,
    /// Current geometry; may be unconfigured or outside the target output.
    pub rect: ScriptRect,
}

/// A snapshot of layout inputs, without compositor objects or handles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptContext {
    /// Available bounded layout area.
    pub area: ScriptRect,
    /// Windows to place, each of which must appear exactly once in the result.
    pub windows: Vec<ScriptWindow>,
    /// Focused window ID, when present in this context.
    pub focused: Option<u64>,
    /// Requested gap in logical pixels.
    pub gaps: i32,
    /// Requested scroll offset; returned layouts must still fit inside `area`.
    pub scroll_offset: i32,
}

/// A validated placement returned by a layout function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptPlacement {
    /// ID of the window being placed.
    pub window: u64,
    /// Bounded positive rectangle in compositor coordinates.
    pub rect: ScriptRect,
}

/// A compiled extension with a resource-limited, capability-free Rhai engine.
pub struct ScriptHost {
    engine: Engine,
    ast: AST,
    call_started: Arc<Mutex<Instant>>,
}

impl ScriptHost {
    /// Read and compile a bounded UTF-8 extension file once.
    pub fn load(path: &Path) -> Result<Self, String> {
        let mut source = String::new();
        File::open(path)
            .and_then(|file| {
                file.take((MAX_SOURCE_BYTES + 1) as u64)
                    .read_to_string(&mut source)
            })
            .map_err(|error| format!("cannot read Rhai extension {}: {error}", path.display()))?;
        Self::from_source(&source).map_err(|error| format!("{}: {error}", path.display()))
    }

    /// Compile functions without running top-level code or exposing native IO.
    pub fn from_source(source: &str) -> Result<Self, String> {
        if source.len() > MAX_SOURCE_BYTES {
            return Err(format!("Rhai extension exceeds {MAX_SOURCE_BYTES} bytes"));
        }
        let mut engine = Engine::new();
        engine.set_module_resolver(DummyModuleResolver::new());
        engine.disable_symbol("import");
        engine.disable_symbol("export");
        engine.disable_symbol("eval");
        engine.set_max_operations(100_000);
        engine.set_max_call_levels(32);
        engine.set_max_expr_depths(64, 32);
        engine.set_max_string_size(16_384);
        engine.set_max_array_size(4096);
        // Rhai counts nested map entries together, including every window DTO.
        engine.set_max_map_size(16_384);
        engine.set_max_variables(256);
        engine.set_max_functions(128);
        install_map_guard(&mut engine);
        let call_started = Arc::new(Mutex::new(Instant::now()));
        let timer = Arc::clone(&call_started);
        engine.on_progress(move |operations| {
            if operations % 1024 == 0
                && timer
                    .lock()
                    .map_or(true, |start| start.elapsed() > Duration::from_millis(250))
            {
                Some(Dynamic::from("extension execution deadline exceeded"))
            } else {
                None
            }
        });
        engine.on_print(|_| {});
        engine.on_debug(|_, _, _| {});
        let ast = engine
            .compile(source)
            .map_err(|error| format!("Rhai compile error: {error}"))?
            .clone_functions_only();
        Ok(Self {
            engine,
            ast,
            call_started,
        })
    }

    /// Call a layout function and validate a complete, bounded window assignment.
    ///
    /// Empty window lists return immediately without invoking script code.
    pub fn layout(
        &mut self,
        name: &str,
        ctx: ScriptContext,
    ) -> Result<Vec<ScriptPlacement>, String> {
        let result = self.layout_inner(name, ctx);
        if let Err(error) = &result {
            eprintln!("clear: Rhai layout {name:?} failed: {error}; use a built-in layout");
        }
        result
    }

    fn layout_inner(
        &mut self,
        name: &str,
        ctx: ScriptContext,
    ) -> Result<Vec<ScriptPlacement>, String> {
        if ctx.windows.is_empty() {
            return Ok(Vec::new());
        }
        validate_function(name)?;
        validate_rect(ctx.area)?;
        if ctx.windows.len() > MAX_WINDOWS {
            return Err(format!("layout exceeds {MAX_WINDOWS} windows"));
        }
        if !(0..=4096).contains(&ctx.gaps) {
            return Err("context gaps must be in 0..=4096".into());
        }
        if i64::from(ctx.scroll_offset).abs() > MAX_COORDINATE {
            return Err("context scroll offset is unreasonable".into());
        }
        let mut expected = BTreeSet::new();
        let mut windows = Array::with_capacity(ctx.windows.len());
        for window in &ctx.windows {
            if !expected.insert(window.id) {
                return Err(format!("duplicate context window {}", window.id));
            }
            let mut map = Map::new();
            map.insert("id".into(), id_dynamic(window.id)?);
            map.insert("rect".into(), rect_dynamic(window.rect));
            windows.push(Dynamic::from_map(map));
        }
        if ctx.focused.is_some_and(|id| !expected.contains(&id)) {
            return Err("focused window is absent from context".into());
        }
        let mut context = Map::new();
        context.insert("area".into(), rect_dynamic(ctx.area));
        context.insert("windows".into(), Dynamic::from_array(windows));
        context.insert(
            "focused".into(),
            match ctx.focused {
                Some(id) => id_dynamic(id)?,
                None => Dynamic::UNIT,
            },
        );
        context.insert("gaps".into(), Dynamic::from_int(INT::from(ctx.gaps)));
        context.insert(
            "scroll_offset".into(),
            Dynamic::from_int(INT::from(ctx.scroll_offset)),
        );
        self.start_call()?;
        let value: Dynamic = self
            .engine
            .call_fn_with_options(
                CallFnOptions::new().eval_ast(false),
                &mut Scope::new(),
                &self.ast,
                name,
                (Dynamic::from_map(context),),
            )
            .map_err(|error| format!("Rhai runtime error: {error}"))?;
        check_maps(&value, 0, &mut 16_384).map_err(|error| error.to_string())?;
        let placements = into_array(value, "layout result")?;
        if placements.len() != expected.len() {
            return Err(format!(
                "layout must place every window exactly once: expected {}, got {}",
                expected.len(),
                placements.len()
            ));
        }
        let mut seen = BTreeSet::new();
        let mut result = Vec::with_capacity(placements.len());
        for placement in placements {
            let mut map = into_map(placement, "placement")?;
            let window = take_u64(&mut map, "window")?;
            if !expected.contains(&window) {
                return Err(format!("layout returned unknown window {window}"));
            }
            if !seen.insert(window) {
                return Err(format!("layout returned duplicate window {window}"));
            }
            let rect = ScriptRect {
                x: take_i32(&mut map, "x")?,
                y: take_i32(&mut map, "y")?,
                width: take_i32(&mut map, "width")?,
                height: take_i32(&mut map, "height")?,
            };
            no_extra_fields(&map)?;
            validate_rect(rect)?;
            if !contains(ctx.area, rect) {
                return Err(format!(
                    "window {window} placement is outside the context area"
                ));
            }
            result.push(ScriptPlacement { window, rect });
        }
        Ok(result)
    }

    /// Call a zero-argument function returning typed requests, without executing them.
    ///
    /// The platform must enforce dispatch policy and bound nested `Script` actions.
    pub fn action(&mut self, name: &str) -> Result<Vec<Action>, String> {
        let result = self.action_inner(name);
        if let Err(error) = &result {
            eprintln!("clear: Rhai action {name:?} failed: {error}; ignoring requests");
        }
        result
    }

    fn start_call(&self) -> Result<(), String> {
        *self
            .call_started
            .lock()
            .map_err(|_| "extension timer lock poisoned")? = Instant::now();
        Ok(())
    }

    fn action_inner(&mut self, name: &str) -> Result<Vec<Action>, String> {
        validate_function(name)?;
        self.start_call()?;
        let value: Dynamic = self
            .engine
            .call_fn_with_options(
                CallFnOptions::new().eval_ast(false),
                &mut Scope::new(),
                &self.ast,
                name,
                (),
            )
            .map_err(|error| format!("Rhai runtime error: {error}"))?;
        check_maps(&value, 0, &mut 16_384).map_err(|error| error.to_string())?;
        let values = into_array(value, "action result")?;
        if values.len() > MAX_ACTIONS {
            return Err(format!("action result exceeds {MAX_ACTIONS} requests"));
        }
        values.into_iter().map(decode_action).collect()
    }
}

#[allow(deprecated)]
fn install_map_guard(engine: &mut Engine) {
    // Rhai 1.26 does not check map limits on every indexed insertion. Check
    // values before subsequent access as well, so in-place growth stays bounded.
    // on_var is marked volatile by Rhai, rather than actually deprecated.
    engine.on_var(|name, _, context| {
        if let Some(value) = context.scope().get(name) {
            check_maps(value, 0, &mut 16_384)?;
        }
        Ok(None)
    });
}

fn check_maps(
    value: &Dynamic,
    depth: usize,
    remaining: &mut usize,
) -> Result<(), Box<rhai::EvalAltResult>> {
    if depth > 32 || *remaining == 0 {
        return Err("extension value nesting or traversal limit exceeded".into());
    }
    *remaining -= 1;
    if let Some(map) = value.read_lock::<Map>() {
        if map.len() > 256 {
            return Err("extension map size limit exceeded".into());
        }
        for value in map.values() {
            check_maps(value, depth + 1, remaining)?;
        }
    } else if let Some(array) = value.read_lock::<Array>() {
        for value in array.iter() {
            check_maps(value, depth + 1, remaining)?;
        }
    }
    Ok(())
}

fn validate_function(name: &str) -> Result<(), String> {
    if valid_function_name(name) {
        Ok(())
    } else {
        Err("invalid Rhai function name".into())
    }
}

fn validate_rect(rect: ScriptRect) -> Result<(), String> {
    let right = i64::from(rect.x) + i64::from(rect.width);
    let bottom = i64::from(rect.y) + i64::from(rect.height);
    if rect.width <= 0
        || rect.height <= 0
        || i64::from(rect.x).abs() > MAX_COORDINATE
        || i64::from(rect.y).abs() > MAX_COORDINATE
        || right.abs() > MAX_COORDINATE
        || bottom.abs() > MAX_COORDINATE
    {
        return Err("rectangles must have positive dimensions and edges within ±1000000".into());
    }
    Ok(())
}

fn contains(area: ScriptRect, rect: ScriptRect) -> bool {
    rect.x >= area.x
        && rect.y >= area.y
        && i64::from(rect.x) + i64::from(rect.width) <= i64::from(area.x) + i64::from(area.width)
        && i64::from(rect.y) + i64::from(rect.height) <= i64::from(area.y) + i64::from(area.height)
}

fn rect_dynamic(rect: ScriptRect) -> Dynamic {
    let mut map = Map::new();
    for (key, value) in [
        ("x", rect.x),
        ("y", rect.y),
        ("width", rect.width),
        ("height", rect.height),
    ] {
        map.insert(key.into(), Dynamic::from_int(INT::from(value)));
    }
    Dynamic::from_map(map)
}

fn id_dynamic(id: u64) -> Result<Dynamic, String> {
    INT::try_from(id)
        .map(Dynamic::from_int)
        .map_err(|_| "window ID exceeds Rhai's signed integer range".into())
}

fn into_array(value: Dynamic, label: &str) -> Result<Array, String> {
    value
        .try_cast::<Array>()
        .ok_or_else(|| format!("{label} must be an array"))
}

fn into_map(value: Dynamic, label: &str) -> Result<Map, String> {
    value
        .try_cast::<Map>()
        .ok_or_else(|| format!("{label} must be a map"))
}

fn take(map: &mut Map, key: &str) -> Result<Dynamic, String> {
    map.remove(key)
        .ok_or_else(|| format!("missing field {key:?}"))
}

fn take_int(map: &mut Map, key: &str) -> Result<INT, String> {
    take(map, key)?
        .try_cast::<INT>()
        .ok_or_else(|| format!("{key} must be an integer"))
}

fn take_i32(map: &mut Map, key: &str) -> Result<i32, String> {
    i32::try_from(take_int(map, key)?).map_err(|_| format!("{key} exceeds i32 range"))
}

fn take_u64(map: &mut Map, key: &str) -> Result<u64, String> {
    u64::try_from(take_int(map, key)?).map_err(|_| format!("{key} must be nonnegative"))
}

fn take_string(map: &mut Map, key: &str) -> Result<String, String> {
    take(map, key)?
        .try_cast::<rhai::ImmutableString>()
        .map(|s| s.to_string())
        .ok_or_else(|| format!("{key} must be a string"))
}

fn no_extra_fields(map: &Map) -> Result<(), String> {
    match map.keys().next() {
        Some(key) => Err(format!("unknown result field {key:?}")),
        None => Ok(()),
    }
}

fn decode_action(value: Dynamic) -> Result<Action, String> {
    let mut map = into_map(value, "action")?;
    let tag = take_string(&mut map, "action")?;
    let action = match tag.as_str() {
        "focus_next" => Action::FocusNext,
        "focus_previous" => Action::FocusPrevious,
        "cycle_output" => Action::CycleOutput,
        "switch_workspace" => Action::SwitchWorkspace {
            workspace: take_u64(&mut map, "workspace")?,
        },
        "move_to_workspace" => Action::MoveToWorkspace {
            workspace: take_u64(&mut map, "workspace")?,
        },
        "move_to_output" => Action::MoveToOutput {
            output: take_u64(&mut map, "output")?,
        },
        "set_workspace_mode" => Action::SetWorkspaceMode {
            mode: take_string(&mut map, "mode")?,
        },
        "set_output_mode" => Action::SetOutputMode {
            mode: take_string(&mut map, "mode")?,
        },
        "clear_output_mode" => Action::ClearOutputMode,
        "cycle_mode" => Action::CycleMode,
        "stretch_all" => Action::StretchAll,
        "unstretch" => Action::Unstretch,
        "toggle_floating" => Action::ToggleFloating,
        "toggle_maximized" => Action::ToggleMaximized,
        "minimize" => Action::Minimize,
        "scroll" => Action::Scroll {
            amount: take_i32(&mut map, "amount")?,
        },
        "close_focused" => Action::CloseFocused,
        "spawn" => {
            let command = into_array(take(&mut map, "command")?, "command")?
                .into_iter()
                .map(|arg| {
                    arg.try_cast::<rhai::ImmutableString>()
                        .map(|s| s.to_string())
                        .ok_or_else(|| "command arguments must be strings".to_string())
                })
                .collect::<Result<Vec<_>, _>>()?;
            Action::Spawn { command }
        }
        "quit" => Action::Quit,
        "reload" => Action::Reload,
        "script" => Action::Script {
            name: take_string(&mut map, "name")?,
        },
        _ => return Err(format!("unsupported action {tag:?}")),
    };
    no_extra_fields(&map)?;
    action.validate()?;
    Ok(action)
}
