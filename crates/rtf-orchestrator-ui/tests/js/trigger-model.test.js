// Unit tests for static/trigger-model.js. Run with `mise run pr-js-test`.
//
// The fixture is the page data the server embeds for the preview test plan, kept in sync by the
// `js_data_matches_the_js_test_fixture` Rust test.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { coerce, createModel } from "../../static/trigger-model.js";

const data = JSON.parse(readFileSync(new URL("./fixtures/test-plan-details.json", import.meta.url)));
const model = createModel(data);

const isScalar = (v) => ["string", "number", "boolean"].includes(typeof v);

test("the page data has the shape the model and charts read", () => {
    for (const v of data.variables) {
        assert.equal(typeof v.name, "string");
        assert.ok(Array.isArray(v.sections));
        assert.equal(typeof v.description, "string");
        assert.ok(v.default.every(isScalar));
        assert.ok(v.allowed === null || v.allowed.every(isScalar));
        assert.equal(typeof v.required, "boolean");
    }
    for (const entries of Object.values(data.compound_groups)) {
        const keys = Object.keys(entries[0]).sort();
        for (const e of entries) {
            assert.deepEqual(Object.keys(e).sort(), keys);
            assert.ok(Object.values(e).every(isScalar));
        }
    }
    assert.equal(typeof data.history.runs_by_day, "object");
    assert.ok(Array.isArray(data.history.execution_durations.bins));
});

test("coerce only converts text that round-trips", () => {
    assert.equal(coerce("10"), 10);
    assert.equal(coerce("-2.5"), -2.5);
    assert.equal(coerce("true"), true);
    assert.equal(coerce("1.3"), 1.3);
    assert.equal(coerce("1.30"), "1.30");
    assert.equal(coerce("007"), "007");
    assert.equal(coerce("60s"), "60s");
});

test("an untouched form submits no overrides and runs the test plan defaults", () => {
    const s = model.freshState();
    assert.deepEqual(model.toOverrides(s), {});
    assert.equal(model.countOf(s), 8);
    assert.equal(model.formulaOf(s), "2 region × 2 colours × 2 subjects");
});

test("one value is a scalar override, several are a matrix dimension", () => {
    const s = model.freshState();
    s.selected.push("duration", "tier");
    s.values.duration = ["90s"];
    s.values.tier = ["free", "paid"];
    assert.deepEqual(model.toOverrides(s), { duration: "90s", tier: ["free", "paid"] });
    assert.equal(model.countOf(s), 16);
});

test("a variable selected but left at its default is not submitted", () => {
    const s = model.freshState();
    s.selected.push("region");
    s.values.region = ["us-east-1", "eu-west-1"];
    assert.deepEqual(model.toOverrides(s), {});
});

test("overrides round-trip through the form state", () => {
    const overrides = {
        duration: "90s",
        tier: ["free", "paid"],
        colours: [],
        subjects: [{ setup_subject: "a", scenario_subject: "b" }],
    };
    const roundTripped = model.toOverrides(model.fromOverrides(overrides));
    assert.deepEqual(roundTripped, { ...overrides, background: "blue", foreground: "red" });
});

test("a scalar for a compound member moves it out of its group", () => {
    const s = model.fromOverrides({ foreground: "green" });
    assert.deepEqual(model.toOverrides(s), {
        foreground: "green",
        colours: [{ background: "blue" }, { background: "white" }],
    });
});

test("adding a variable to a group keeps the executions; removing it fixes its first value", () => {
    const s = model.freshState();
    model.addToGroup(s, "colours", "region");
    assert.equal(s.groups.colours.entries.length, 4);
    assert.equal(model.countOf(s), 8);

    assert.deepEqual(model.removeFromGroup(s, "colours", "region"), ["us-east-1"]);
    assert.equal(s.groups.colours.entries.length, 2);
    assert.equal(model.countOf(s), 4);
});

test("problems block triggering an empty variable or a mixed-type column", () => {
    const s = model.freshState();
    s.selected.push("duration");
    s.values.duration = [];
    s.groups.colours.entries[0].background = 1;
    s.values.worker_overlay = ["worker.yaml"];
    assert.deepEqual(model.problemsOf(s), ["duration has no values", "colours.background mixes value types"]);
    assert.equal(model.countOf(s), 0);
});

test("a required variable starts selected with no values and blocks triggering", () => {
    const s = model.freshState();
    assert.ok(s.selected.includes("worker_overlay"));
    assert.deepEqual(s.values.worker_overlay, []);
    assert.deepEqual(model.problemsOf(s), ["worker_overlay is required"]);
    assert.equal(model.countOf(s), 8);
});

test("a required variable with a value is submitted and no longer blocks triggering", () => {
    const s = model.freshState();
    s.values.worker_overlay = ["worker.yaml"];
    assert.deepEqual(model.problemsOf(s), []);
    assert.deepEqual(model.toOverrides(s), { worker_overlay: "worker.yaml" });
});

test("loading overrides that omit a required variable keeps it selected", () => {
    const s = model.fromOverrides({ duration: "90s" });
    assert.ok(s.selected.includes("worker_overlay"));
    assert.deepEqual(model.problemsOf(s), ["worker_overlay is required"]);
});

test("a required variable moved into a group without a value blocks triggering", () => {
    const s = model.freshState();
    model.addToGroup(s, "colours", "worker_overlay");
    assert.deepEqual(model.problemsOf(s), ["colours.worker_overlay is required"]);
});
