// Desktop IDs stay strings: a Rust u64 need not fit in a JavaScript number.
var builtinModes = ["floating", "scrolling", "master_stack", "columns", "rows", "grid", "spiral", "monocle"];

function encodeRequest(id, request) {
    return JSON.stringify({version: 1, id: id, request: request}) + "\n";
}

function decodeMessage(line) {
    var message = JSON.parse(line);
    if (!message || message.version !== 1)
        throw new Error("Unsupported Clear IPC version");
    if (message.type === "snapshot" || message.type === "state") {
        var state = message.state;
        if (!state || !Array.isArray(state.outputs) || !Array.isArray(state.workspaces)
                || !Array.isArray(state.groups) || !Array.isArray(state.windows))
            throw new Error("Invalid Clear state");
    } else if (message.type === "animation_panels") {
        if (!Array.isArray(message.panels) || !message.panels.every(function(panel) {
            return typeof panel.panel === "string" && typeof panel.output === "string"
                && typeof panel.namespace === "string" && Number.isInteger(panel.width)
                && Number.isInteger(panel.height) && panel.width > 0 && panel.height > 0;
        })) throw new Error("Invalid Clear animation panels");
    } else if (message.type === "error") {
        if (typeof message.message !== "string")
            throw new Error("Invalid Clear error");
    } else if (message.type !== "ok") {
        throw new Error("Unknown Clear message type");
    }
    return message;
}

function outputForScreen(state, name) {
    if (!state || !name)
        return null;
    return state.outputs.find(function(output) { return output.name === name; }) || null;
}

function groupForOutput(state, output) {
    if (!state || !output)
        return null;
    return state.groups.find(function(group) {
        return group.outputs.indexOf(output.id) !== -1;
    }) || null;
}

function focusedTitle(state, output) {
    var group = groupForOutput(state, output);
    if (!group || state.focused_output === null
            || group.outputs.indexOf(state.focused_output) === -1)
        return "";
    // `window.output` is a saved home, not current presentation or visibility.
    var window = state.windows.find(function(candidate) {
        return candidate.id === state.focused_window && candidate.workspace === group.workspace;
    });
    return window ? (window.title || window.app_id || "Untitled") : "";
}

function nextMode(current) {
    return builtinModes[(builtinModes.indexOf(current) + 1) % builtinModes.length];
}

function visibleWindows(state, output) {
    var group = groupForOutput(state, output);
    if (!group) return [];
    return state.windows.filter(function(window) {
        return window.workspace === group.workspace && window.role === "normal" && !window.minimized;
    });
}

function appGroups(state, output, pins) {
    var groups = [];
    var byId = {};
    (pins || []).forEach(function(id) {
        if (typeof id !== "string" || !id || byId[id]) return;
        var group = {id: id, windows: [], pinned: true};
        byId[id] = group;
        groups.push(group);
    });
    (state ? state.windows : []).filter(function(window) {
        return window.role === "normal";
    }).forEach(function(window) {
        var id = window.app_id || "unknown";
        var group = byId[id];
        if (!group) {
            group = {id: id, windows: [], pinned: false};
            byId[id] = group;
            groups.push(group);
        }
        group.windows.push(window);
    });
    return groups;
}

function modeIcon(mode) {
    return "icons/" + (builtinModes.indexOf(mode) < 0 ? "script" : mode) + ".svg";
}

function workspaceNumber(output) {
    return output ? output.workspace : "–";
}

function matchesApp(entry, query) {
    var needle = (query || "").trim().toLocaleLowerCase();
    if (!needle) return true;
    return [entry.name, entry.genericName, entry.id]
        .concat(entry.keywords || [])
        .some(function(value) { return String(value || "").toLocaleLowerCase().indexOf(needle) !== -1; });
}

function supportsAnimationTargets(state) {
    return !!state && Array.isArray(state.capabilities)
        && state.capabilities.indexOf("animation_targets_v1") !== -1;
}

function animationPanelFor(panels, output, namespace, width, height) {
    if (!output || !namespace) return null;
    var matches = (panels || []).filter(function(panel) {
        return panel.output === output.id && panel.namespace === namespace
            && panel.width === width && panel.height === height;
    });
    // Ambiguous same-process panels cannot prove which QML window owns an icon.
    return matches.length === 1 ? matches[0] : null;
}

function animationAppTargets(state, output, icons, viewport, panelWidth, panelHeight) {
    if (!state || !output || !viewport) return [];
    var targets = [];
    var seen = Object.create(null);
    (icons || []).forEach(function(icon) {
        var values = [icon.x, icon.y, icon.width, icon.height,
            viewport.x, viewport.y, viewport.width, viewport.height];
        if (!values.every(function(value) { return typeof value === "number" && Number.isFinite(value); })
                || icon.width <= 0 || icon.height <= 0 || targets.length === 128
                || typeof icon.app_id !== "string" || !icon.app_id || seen[icon.app_id]) return;
        if (encodeURIComponent(icon.app_id).replace(/%[0-9A-F]{2}|./g, "x").length > 256) return;
        if (!state.windows.some(function(window) {
            return window.role === "normal" && window.output === output.id && window.app_id === icon.app_id;
        })) return;
        // Exclude clipped icons rather than reporting a target for an invisible
        // portion. Coordinates come from actual delegates after layout/scroll.
        if (icon.x < viewport.x || icon.y < viewport.y
                || icon.x + icon.width > viewport.x + viewport.width
                || icon.y + icon.height > viewport.y + viewport.height) return;
        var x = Math.floor(icon.x), y = Math.floor(icon.y);
        var width = Math.ceil(icon.x + icon.width) - x;
        var height = Math.ceil(icon.y + icon.height) - y;
        if (x < 0 || y < 0 || x + width > panelWidth || y + height > panelHeight) return;
        seen[icon.app_id] = true;
        targets.push({app_id: icon.app_id, rect: {x: x, y: y, width: width, height: height}});
    });
    return targets;
}
