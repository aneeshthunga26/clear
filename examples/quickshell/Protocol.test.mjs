import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import vm from "node:vm";

// Run the same dependency-free JavaScript that QML imports; not a Socket/QML test.
const protocol = vm.createContext({});
vm.runInContext(readFileSync(new URL("Protocol.js", import.meta.url), "utf8"), protocol);

const state = {
    outputs: [
        { id: "9007199254740993", name: "Virtual-1", workspace: "1", effective_mode: "columns", mode_override: "columns" },
        { id: "2", name: "Virtual-2", workspace: "1", effective_mode: "floating", mode_override: null },
        { id: "3", name: "Virtual-3", workspace: "2", effective_mode: "monocle", mode_override: null },
    ],
    workspaces: [{ id: "1", name: "One" }, { id: "2", name: "Two" }],
    groups: [{ outputs: ["9007199254740993", "2"], workspace: "1" }, { outputs: ["3"], workspace: "2" }],
    windows: [{ id: "18446744073709551615", title: "Editor\nnotes", app_id: "editor", workspace: "1", output: "3", focused: true }],
    focused_output: "2",
    focused_window: "18446744073709551615",
};

function copyState() {
    return structuredClone(state);
}

test("newline requests preserve string IDs, u32 request IDs, and exact fields", () => {
    const requests = [
        { type: "snapshot" },
        { type: "subscribe" },
        { type: "focus_output", output: state.outputs[0].id },
        { type: "switch_workspace", output: "2", workspace: "1" },
        { type: "focus_window", window: state.focused_window },
        { type: "set_maximized", window: state.focused_window, maximized: true },
        { type: "set_minimized", window: state.focused_window, minimized: false },
        { type: "set_mode", output: "2", mode: "script:columns" },
        { type: "clear_mode", output: "2" },
        { type: "stretch", output: "2" },
        { type: "split", output: "2" },
    ];
    for (const request of requests) {
        const line = protocol.encodeRequest(4294967295, request);
        assert.ok(line.endsWith("\n"));
        assert.equal(line.split("\n").length, 2);
        assert.deepEqual(JSON.parse(line), { version: 1, id: 4294967295, request });
    }
});

test("minimized windows stay in the dock but not the visible-window list", () => {
    const s = copyState();
    s.windows[0].role = "normal";
    s.windows[0].minimized = true;
    s.windows[0].maximized = true;
    assert.equal(protocol.visibleWindows(s, s.outputs[0]).length, 0);
    const groups = protocol.appGroups(s, s.outputs[0], []);
    assert.equal(groups.length, 1);
    assert.equal(groups[0].windows[0].minimized, true);
    assert.equal(groups[0].windows[0].maximized, true);
});

test("snapshot and full state update use the same state shape", () => {
    for (const type of ["snapshot", "state"]) {
        const message = { version: 1, type, state };
        if (type === "snapshot") message.id = 1;
        assert.equal(JSON.stringify(protocol.decodeMessage(JSON.stringify(message))), JSON.stringify(message));
    }
});

test("ok and errors use message, including null error request ID", () => {
    assert.equal(protocol.decodeMessage('{"version":1,"type":"ok","id":2}').id, 2);
    const error = protocol.decodeMessage('{"version":1,"type":"error","id":null,"message":"unknown output"}');
    assert.equal(error.message, "unknown output");
    assert.equal(error.id, null);
});

test("malformed, unknown, and incompatible messages are rejected", () => {
    for (const line of ["", "{", "null", '{"version":2,"type":"ok"}', '{"version":1,"type":"other"}', '{"version":1,"type":"state","state":{}}', '{"version":1,"type":"error","error":"wrong field"}']) {
        assert.throws(() => protocol.decodeMessage(line));
    }
});

test("screens match exact names, never positions, IDs, or the globally focused output", () => {
    assert.equal(protocol.outputForScreen(state, "Virtual-1"), state.outputs[0]);
    for (const name of ["", "virtual-1", "9007199254740993", "unknown"]) {
        assert.equal(protocol.outputForScreen(state, name), null);
    }
    assert.equal(protocol.outputForScreen(null, "Virtual-1"), null);
});

test("focused title repeats on stretched group members, not the saved home output", () => {
    assert.equal(protocol.focusedTitle(state, state.outputs[0]), "Editor\nnotes");
    assert.equal(protocol.focusedTitle(state, state.outputs[1]), "Editor\nnotes");
    assert.equal(protocol.focusedTitle(state, state.outputs[2]), "");
    assert.equal(protocol.focusedTitle(null, null), "");
});

test("missing/hidden focus never leaks a title onto a different presented workspace", () => {
    for (const patch of [{ focused_output: null }, { focused_window: null }, { focused_window: "missing" }, { focused_output: "3" }]) {
        assert.equal(protocol.focusedTitle({ ...state, ...patch }, state.outputs[0]), "");
    }
    const hidden = copyState();
    hidden.windows[0].workspace = "2";
    assert.equal(protocol.focusedTitle(hidden, hidden.outputs[0]), "");
});

test("empty titles fall back to app ID then Untitled", () => {
    const empty = copyState();
    empty.windows[0].title = "";
    assert.equal(protocol.focusedTitle(empty, empty.outputs[0]), "editor");
    empty.windows[0].app_id = "";
    assert.equal(protocol.focusedTitle(empty, empty.outputs[0]), "Untitled");
});

test("mode cycle uses canonical names; script mode enters the built-in cycle", () => {
    const modes = ["floating", "scrolling", "master_stack", "columns", "rows", "grid", "spiral", "monocle"];
    for (let i = 0; i < modes.length; i++) {
        assert.equal(protocol.nextMode(modes[i]), modes[(i + 1) % modes.length]);
    }
    assert.equal(protocol.nextMode("script:columns"), "floating");
});

test("dock groups pinned and all open app windows across workspaces", () => {
    const expanded = copyState();
    expanded.windows.push({ id: "2", title: "Other", app_id: "editor", workspace: "2", role: "normal" });
    expanded.windows[0].role = "normal";
    const groups = protocol.appGroups(expanded, expanded.outputs[0], ["browser", "editor"]);
    assert.equal(groups.length, 2);
    assert.equal(groups[0].id, "browser");
    assert.equal(groups[0].windows.length, 0);
    assert.equal(groups[1].windows.length, 2);
    assert.equal(groups[1].pinned, true);
});

test("mode icons and launcher search have bounded, predictable mapping", () => {
    assert.equal(protocol.modeIcon("columns"), "icons/columns.svg");
    for (const mode of ["rows", "grid", "spiral"])
        assert.equal(protocol.modeIcon(mode), "icons/" + mode + ".svg");
    assert.equal(protocol.modeIcon("script:custom"), "icons/script.svg");
    const entry = { name: "Text Editor", genericName: "Notes", id: "editor", keywords: ["writing"] };
    assert.equal(protocol.matchesApp(entry, "WRITE"), false);
    assert.equal(protocol.matchesApp(entry, "writ"), true);
    assert.equal(protocol.matchesApp(entry, " notes "), true);
});

test("animation targets negotiate capability and require unambiguous committed panel dimensions", () => {
    assert.equal(protocol.supportsAnimationTargets(state), false);
    assert.equal(protocol.supportsAnimationTargets({...state, capabilities: ["animation_targets_v1"]}), true);
    const output = {id: "1"};
    const panels = [{panel: "18446744073709551615", output: "1", namespace: "panel", width: 640, height: 32}];
    assert.equal(protocol.animationPanelFor(panels, output, "panel", 640, 32).panel, panels[0].panel);
    assert.equal(protocol.animationPanelFor(panels, output, "panel", 600, 32), null);
    assert.equal(protocol.animationPanelFor([...panels, {...panels[0], panel: "2"}], output, "panel", 640, 32), null);
    const response = protocol.decodeMessage(JSON.stringify({version: 1, type: "animation_panels", id: 1, panels}));
    assert.equal(response.panels[0].panel, "18446744073709551615");
});

test("animation icon reports use actual panel-local bounds, exclude clipping and foreign app groups", () => {
    const model = {...state, windows: [{id: "1", app_id: "editor", role: "normal", output: "1"}]};
    const viewport = {x: 10, y: 0, width: 100, height: 32};
    const icons = [
        {app_id: "editor", x: 10.5, y: 6, width: 20, height: 20},
        {app_id: "editor", x: 50, y: 6, width: 20, height: 20},
        {app_id: "unmapped-pin", x: 50, y: 6, width: 20, height: 20},
    ];
    const targets = JSON.parse(JSON.stringify(protocol.animationAppTargets(model, {id: "1"}, icons, viewport, 640, 32)));
    assert.deepEqual(targets, [{app_id: "editor", rect: {x: 10, y: 6, width: 21, height: 20}}]);
    for (const x of [-10, 100, Infinity, NaN]) {
        assert.equal(protocol.animationAppTargets(model, {id: "1"}, [{...icons[0], x}], viewport, 640, 32).length, 0);
    }
    assert.equal(protocol.animationAppTargets(model, {id: "2"}, icons, viewport, 640, 32).length, 0);
});
