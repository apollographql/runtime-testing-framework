// The test plan detail page's client-side behaviour: the trigger form (an Alpine.js component whose
// markup lives in templates/test_plan_detail.html) and the two history charts. Everything else on
// this page is plain server-rendered HTML/links needing no JS.
//
// Data comes from the `#test-plan-details-data` script tag the template embeds - see
// `TestPlanDetailsView::js_data` in src/view/test_plan_details.rs for exactly what's in it.
//
// The form state is a set of overrides on top of the test plan's defaults:
// - `selected`/`values`: standalone variables the user has chosen to change, and their values
//   (one value = fixed, several = a matrix dimension).
// - `groups`: the effective compound dimensions, starting as a copy of the test plan's.
// `toOverrides` turns that state into the `VariableOverride` JSON the trigger endpoint expects, and
// `fromOverrides` goes the other way for Re-run links and rejected triggers.
//
// This file must load before alpine.min.js so the component is registered before Alpine starts.
(function() {
    const dataEl = document.getElementById("test-plan-details-data");
    if (!dataEl) return;
    const data = JSON.parse(dataEl.textContent);

    // ==========================================================================
    // Override model
    // ==========================================================================

    const VARS = Object.fromEntries(data.variables.map((v) => [v.name, v]));
    const PLAN_GROUPS = Object.fromEntries(Object.entries(data.compound_groups).map(([name, entries]) => [
        name,
        { vars: entries.length ? Object.keys(entries[0]) : [], entries },
    ]));
    const PLAN_GROUP_MEMBERS = new Set(Object.values(PLAN_GROUPS).flatMap((g) => g.vars));
    const EMPTY_GROUP = { vars: [], entries: [] };

    const clone = (o) => JSON.parse(JSON.stringify(o));
    const fmt = (v) => (typeof v === "string" ? v : JSON.stringify(v));
    const canon = (o) => {
        if (Array.isArray(o)) return o.map(canon);
        if (o && typeof o === "object") return Object.fromEntries(Object.keys(o).sort().map((k) => [k, canon(o[k])]));
        return o;
    };
    const sameJSON = (a, b) => JSON.stringify(canon(a)) === JSON.stringify(canon(b));

    function short(v) {
        const s = fmt(v);
        const first = s.split("\n")[0];
        return first.length > 40 || s.includes("\n") ? `${first.slice(0, 40)}…` : s;
    }

    // Only text that round-trips exactly becomes a number, so e.g. "1.30" or "007" stay strings and
    // an input's displayed text never changes under the cursor while typing.
    function coerce(s) {
        s = String(s).trim();
        if (s === "true") return true;
        if (s === "false") return false;
        if (/^-?\d+(\.\d+)?$/.test(s) && String(Number(s)) === s) return Number(s);
        return s;
    }

    function dedupe(rows) {
        const seen = new Set();
        return rows.filter((r) => {
            const k = JSON.stringify(canon(r));
            if (seen.has(k)) return false;
            seen.add(k);
            return true;
        });
    }

    const freshState = () => ({ selected: [], values: {}, groups: clone(PLAN_GROUPS) });
    const defaults = (n) => (VARS[n] ? VARS[n].default : null);
    const hasDefault = (n) => (defaults(n) || []).length > 0;

    function groupOf(s, n) {
        for (const [g, d] of Object.entries(s.groups)) if (d.vars.includes(n)) return g;
        return null;
    }

    const allNames = (s) => [...new Set([...data.variables.map((v) => v.name), ...s.selected])];
    const standaloneVars = (s) => allNames(s).filter((n) => !groupOf(s, n));
    const valuesOf = (s, n) => (s.selected.includes(n) ? s.values[n] || [] : defaults(n) || []);

    function isChanged(s, n) {
        if (!s.selected.includes(n)) return false;
        return hasDefault(n) ? !sameJSON(s.values[n], defaults(n)) : s.values[n].length > 0;
    }

    // A variable with no value at all is simply unset: it doesn't multiply the executions.
    function rowsFor(s, n) {
        const vals = valuesOf(s, n);
        if (!vals.length && !(s.selected.includes(n) && hasDefault(n))) return [{}];
        return vals.map((v) => ({ [n]: v }));
    }

    function dims(s) {
        const out = standaloneVars(s).map((n) => ({ label: n, cols: [n], rows: rowsFor(s, n), group: false }));
        for (const [g, d] of Object.entries(s.groups)) out.push({ label: g, cols: d.vars, rows: d.entries, group: true });
        return out;
    }

    const countOf = (s) => dims(s).reduce((t, d) => t * d.rows.length, 1);

    function formulaOf(s) {
        const parts = dims(s).filter((d) => d.rows.length !== 1 || d.group).map((d) => `${d.rows.length} ${d.label}`);
        return parts.join(" × ") || "1 execution";
    }

    function variantsOf(s, limit = 500) {
        const ds = dims(s).filter((d) => d.rows.length !== 1 || d.group);
        let acc = [{}];
        for (const d of ds) {
            const next = [];
            for (const a of acc) {
                for (const r of d.rows) next.push({ ...a, ...r });
                if (next.length >= limit) break;
            }
            acc = next.slice(0, limit);
        }
        return { cols: ds.flatMap((d) => d.cols), rows: acc };
    }

    function problemsOf(s) {
        const p = [];
        for (const n of standaloneVars(s)) {
            const vals = valuesOf(s, n);
            if (!vals.length && s.selected.includes(n) && hasDefault(n)) p.push(`${n} has no values`);
            else if (new Set(vals.map((v) => typeof v)).size > 1) p.push(`${n} mixes value types`);
        }
        for (const [g, d] of Object.entries(s.groups)) {
            if (!d.entries.length) p.push(`${g} has no entries`);
            for (const v of d.vars) {
                if (new Set(d.entries.map((e) => typeof e[v])).size > 1) p.push(`${g}.${v} mixes value types`);
            }
        }
        return p;
    }

    const groupEntries = (d) => d.entries.map((e) => Object.fromEntries(d.vars.map((v) => [v, e[v]])));

    function toOverrides(s) {
        const out = {};
        for (const n of s.selected) {
            if (groupOf(s, n) || !isChanged(s, n)) continue;
            const vals = s.values[n];
            out[n] = vals.length === 1 ? vals[0] : vals;
        }
        for (const g of Object.keys(PLAN_GROUPS)) if (!s.groups[g]) out[g] = [];
        for (const [g, d] of Object.entries(s.groups)) {
            const entries = groupEntries(d);
            if (PLAN_GROUPS[g] && sameJSON(PLAN_GROUPS[g].entries, entries)) continue;
            out[g] = entries;
        }
        return out;
    }

    function fromOverrides(obj) {
        const s = freshState();
        const isGroupValue = (k, v) =>
            PLAN_GROUPS[k] || (Array.isArray(v) && v.length && v[0] && typeof v[0] === "object");

        for (const [k, v] of Object.entries(obj)) {
            if (!isGroupValue(k, v)) continue;
            if (Array.isArray(v) && v.length === 0) {
                if (s.groups[k]) dissolveGroup(s, k);
                continue;
            }
            const vars = Object.keys(v[0]);
            for (const n of vars) {
                const g = groupOf(s, n);
                if (g && g !== k) removeFromGroup(s, g, n);
            }
            s.selected = s.selected.filter((n) => !vars.includes(n));
            s.groups[k] = { vars, entries: clone(v) };
        }
        for (const [k, v] of Object.entries(obj)) {
            if (isGroupValue(k, v)) continue;
            const g = groupOf(s, k);
            if (g) removeFromGroup(s, g, k);
            if (!s.selected.includes(k)) s.selected.push(k);
            s.values[k] = Array.isArray(v) ? v : [v];
        }
        return s;
    }

    function parseOverrides(raw) {
        const obj = raw.trim() ? JSON.parse(raw) : {};
        if (typeof obj !== "object" || Array.isArray(obj) || obj === null) throw new Error("must be a JSON object");
        return obj;
    }

    // Adding a variable multiplies the entries out by its values, so the executions are unchanged
    // until the user deletes the rows they don't want.
    function addToGroup(s, g, n) {
        const vals = valuesOf(s, n).slice();
        const d = s.groups[g];
        d.entries = d.entries.flatMap((e) => (vals.length ? vals : [""]).map((v) => ({ ...e, [n]: v })));
        d.vars.push(n);
        s.selected = s.selected.filter((x) => x !== n);
        delete s.values[n];
        return vals.length;
    }

    // A variable removed from a group becomes fixed at its first entry's value.
    function removeFromGroup(s, g, n) {
        const d = s.groups[g];
        const col = d.entries.map((e) => e[n]);
        d.vars = d.vars.filter((x) => x !== n);
        d.entries.forEach((e) => delete e[n]);
        d.entries = dedupe(d.entries);
        if (!d.vars.length) delete s.groups[g];
        if (!s.selected.includes(n)) s.selected.push(n);
        s.values[n] = col.length ? [col[0]] : [""];
        return s.values[n];
    }

    function dissolveGroup(s, g) {
        for (const n of [...s.groups[g].vars]) removeFromGroup(s, g, n);
    }

    function createGroup(s, name, names) {
        s.groups[name] = { vars: [], entries: [{}] };
        names.forEach((n) => addToGroup(s, name, n));
    }

    function resetGroup(s, g) {
        const plan = PLAN_GROUPS[g];
        if (s.groups[g]) {
            for (const n of [...s.groups[g].vars]) if (!plan.vars.includes(n)) removeFromGroup(s, g, n);
        }
        for (const n of plan.vars) {
            const other = groupOf(s, n);
            if (other && other !== g) removeFromGroup(s, other, n);
            s.selected = s.selected.filter((x) => x !== n);
            delete s.values[n];
        }
        s.groups[g] = clone(plan);
    }

    // ==========================================================================
    // Alpine component
    // ==========================================================================

    document.addEventListener("alpine:init", () => {
        Alpine.data("triggerForm", () => {
            // Read before Alpine binds `x-model` over it: variables rendered by the server (a "Re-run"
            // link, or a rejected trigger) seed the form.
            const initialRaw = document.getElementById("variables").value;
            const forbidden = document.getElementById("trigger-btn").hasAttribute("data-forbidden");
            let toastTimer;

            return {
                ...freshState(),
                pickerOpen: false,
                pickerQuery: "",
                editing: null,
                editText: "",
                newGroup: { open: false, name: "", picked: [] },
                rawText: "",
                // Set while the raw JSON textarea holds content that hasn't been applied to the form
                // (invalid JSON from a rejected trigger, or hand edits): it is left alone and
                // triggering is blocked.
                rawPending: false,
                jsonError: "",
                loadError: false,
                toastMessage: "",
                toastShow: false,
                variants: { total: 0, subtitle: "", cols: [], rows: [] },

                init() {
                    if (!initialRaw.trim()) return;
                    try {
                        Object.assign(this, fromOverrides(parseOverrides(initialRaw)));
                    } catch (err) {
                        this.rawText = initialRaw;
                        this.rawPending = true;
                        this.loadError = true;
                        this.jsonError = `Invalid JSON: ${err.message}`;
                    }
                },

                fmt,
                short,
                coerce,

                // --- derived state ---
                get standalone() { return standaloneVars(this); },
                get changedRows() { return this.selected.filter((n) => !groupOf(this, n)); },
                get nChanged() { return this.changedRows.filter((n) => isChanged(this, n)).length; },
                get defaultRows() { return standaloneVars(this).filter((n) => !this.selected.includes(n)).map((n) => VARS[n]); },
                get groupNames() { return Object.keys(this.groups); },
                get deletedGroups() { return Object.keys(PLAN_GROUPS).filter((g) => !this.groups[g]); },
                get total() { return countOf(this); },
                get formula() { return formulaOf(this); },
                get overridesJson() {
                    const overrides = toOverrides(this);
                    return Object.keys(overrides).length ? JSON.stringify(overrides, null, 2) : "";
                },
                get problems() {
                    const p = problemsOf(this);
                    if (this.rawPending) p.unshift("apply the raw variables JSON to the form first");
                    return p;
                },
                get canTrigger() { return !forbidden && this.total > 0 && !this.problems.length; },
                get pickerItems() {
                    const q = this.pickerQuery.toLowerCase();
                    return data.variables.filter((v) => !this.selected.includes(v.name)
                        && (v.name.toLowerCase().includes(q) || v.description.toLowerCase().includes(q)));
                },

                // --- per-variable helpers ---
                valuesFor(n) { return valuesOf(this, n); },
                isDeclared(n) { return n in VARS; },
                isFreed(n) { return !VARS[n] && PLAN_GROUP_MEMBERS.has(n); },
                sectionsOf(n) { return VARS[n] ? VARS[n].sections : []; },
                descriptionOf(n) { return VARS[n] ? VARS[n].description : ""; },
                allowedOf(n) { return VARS[n] ? VARS[n].allowed : null; },
                hasValue(n, v) { return this.valuesFor(n).some((x) => x === v); },
                invalidValues(n) { return this.valuesFor(n).filter((x) => !this.allowedOf(n).includes(x)); },
                defaultTitle(n) { return (defaults(n) || []).map(fmt).join(", "); },
                defaultLabel(n) {
                    if (!VARS[n]) return this.isFreed(n) ? "from a compound dimension" : "not declared at this ref";
                    const def = defaults(n);
                    return def.length ? `default: ${def.map(short).join(", ")}` : "not set by default";
                },
                revertTitle(n) {
                    if (this.isFreed(n)) return "Needs a value: it has no default outside its compound dimension";
                    return VARS[n] ? "Revert to default" : "Remove";
                },

                // --- variable editing ---
                selectVar(n) {
                    if (!this.selected.includes(n)) {
                        this.values[n] = clone(defaults(n) || []);
                        this.selected.push(n);
                    }
                    this.pickerOpen = false;
                    this.pickerQuery = "";
                    this.$nextTick(() => {
                        const input = document.querySelector(`[data-add-input][data-var="${CSS.escape(n)}"]`);
                        if (input) input.focus();
                    });
                },
                revert(n) {
                    this.selected = this.selected.filter((x) => x !== n);
                    delete this.values[n];
                },
                setValues(n, vals) { this.values[n] = vals; },
                toggleAllowed(n, opt) {
                    const vals = this.valuesFor(n);
                    const allowed = this.allowedOf(n);
                    this.setValues(n, this.hasValue(n, opt)
                        ? vals.filter((x) => x !== opt)
                        : allowed.filter((a) => vals.includes(a) || a === opt));
                },
                addValues(n, input) {
                    const parts = input.value.split(",").map((x) => x.trim()).filter(Boolean).map(coerce);
                    input.value = "";
                    if (parts.length) this.setValues(n, [...this.valuesFor(n), ...parts]);
                },
                removeValue(n, i) { this.setValues(n, this.valuesFor(n).filter((_, j) => j !== i)); },
                removeExact(n, v) { this.setValues(n, this.valuesFor(n).filter((x) => x !== v)); },
                removeLast(n) { this.setValues(n, this.valuesFor(n).slice(0, -1)); },
                isEditing(n, i) { return !!this.editing && this.editing.n === n && this.editing.i === i; },
                startEdit(n, i) {
                    const v = this.valuesFor(n)[i];
                    this.editText = fmt(v);
                    this.editing = { n, i, multiline: typeof v === "string" && v.includes("\n") };
                },
                commitEdit() {
                    if (!this.editing) return;
                    const { n, i, multiline } = this.editing;
                    this.editing = null;
                    const next = [...this.valuesFor(n)];
                    if (this.editText.trim() === "") next.splice(i, 1);
                    else next[i] = multiline ? this.editText : coerce(this.editText);
                    this.setValues(n, next);
                },
                cancelEdit() { this.editing = null; },
                togglePicker() {
                    this.pickerOpen = !this.pickerOpen;
                    if (this.pickerOpen) this.$nextTick(() => this.$refs.pickerQuery.focus());
                },

                // --- compound dimensions ---
                group(g) { return this.groups[g] || EMPTY_GROUP; },
                isPlanGroup(g) { return g in PLAN_GROUPS; },
                groupChanged(g) {
                    return !this.groups[g] || !PLAN_GROUPS[g] || !sameJSON(PLAN_GROUPS[g].entries, groupEntries(this.groups[g]));
                },
                groupStatus(g) {
                    if (!this.isPlanGroup(g)) return "new";
                    return this.groupChanged(g) ? "from test plan · modified" : "from test plan";
                },
                addToGroup(g, n) {
                    if (!n) return;
                    const k = addToGroup(this, g, n);
                    if (k > 1) this.toast(`${n} had ${k} values: entries were multiplied out so the same executions still run`);
                },
                removeFromGroup(g, n) {
                    const v = removeFromGroup(this, g, n);
                    this.toast(`${n} is now fixed at ${fmt(v[0])}`);
                },
                dissolveGroup(g) {
                    dissolveGroup(this, g);
                    this.toast(`${g} deleted — its variables are now fixed values you can edit above`);
                },
                resetGroup(g) { resetGroup(this, g); },
                addEntry(g) {
                    const d = this.groups[g];
                    d.entries.push(d.entries.length ? clone(d.entries[d.entries.length - 1]) : Object.fromEntries(d.vars.map((v) => [v, ""])));
                },
                duplicateEntry(g, i) { this.groups[g].entries.splice(i + 1, 0, clone(this.groups[g].entries[i])); },
                deleteEntry(g, i) { this.groups[g].entries.splice(i, 1); },
                openNewGroup() {
                    this.newGroup = { open: true, name: "", picked: [] };
                    this.$nextTick(() => this.$refs.newGroupName.focus());
                },
                createGroup() {
                    const name = this.newGroup.name.trim().replace(/\s+/g, "_");
                    if (!name) return this.toast("Give the dimension a name");
                    if (this.groups[name] || VARS[name] || PLAN_GROUPS[name]) return this.toast(`${name} is already in use`);
                    if (!this.newGroup.picked.length) return this.toast("Pick at least one variable");
                    createGroup(this, name, this.newGroup.picked);
                    this.newGroup.open = false;
                },

                // --- raw JSON, preview and feedback ---
                applyJson() {
                    try {
                        Object.assign(this, fromOverrides(parseOverrides(this.rawText)));
                        this.rawPending = false;
                        this.loadError = false;
                        this.jsonError = "";
                        this.toast("Form populated from JSON");
                    } catch (err) {
                        this.jsonError = `Invalid JSON: ${err.message}`;
                    }
                },
                showVariants() {
                    const { cols, rows } = variantsOf(this);
                    const total = this.total;
                    this.variants = {
                        total,
                        cols,
                        rows,
                        subtitle: `${this.formula}. Only varying variables are shown${rows.length < total ? `; first ${rows.length} listed` : ""}.`,
                    };
                    this.$refs.variantsDialog.showModal();
                },
                toast(message) {
                    this.toastMessage = message;
                    this.toastShow = true;
                    clearTimeout(toastTimer);
                    toastTimer = setTimeout(() => { this.toastShow = false; }, 3500);
                },
            };
        });
    });

    // ==========================================================================
    // History charts
    // ==========================================================================

    function cssVar(name) {
        return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
    }

    function formatMinSec(totalSeconds) {
        const minutes = Math.floor(totalSeconds / 60);
        const seconds = totalSeconds % 60;
        return `${minutes}:${String(seconds).padStart(2, "0")}`;
    }

    function chartColors() {
        return {
            good: cssVar("--status-success-text"),
            failed: cssVar("--status-failed-text"),
            unrunnable: cssVar("--status-unrunnable-text"),
            accent: cssVar("--color-brand-accent"),
            muted: cssVar("--color-text-secondary"),
            grid: cssVar("--color-border-primary"),
            surface: cssVar("--color-bg-primary"),
            text: cssVar("--color-text-primary"),
        };
    }

    let statusChart, durationChart;

    function renderStatusChart() {
        const c = chartColors();
        const ctx = document.getElementById("chart-status");
        if (!ctx) return;
        const days = Object.keys(data.history.runs_by_day);
        const months = ["", "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
        const labels = days.map((d) => {
            const [, m, dd] = d.split("-");
            return `${months[parseInt(m, 10)]} ${parseInt(dd, 10)}`;
        });
        const successful = days.map((d) => data.history.runs_by_day[d].successful);
        const failed = days.map((d) => data.history.runs_by_day[d].failed);
        const unrunnable = days.map((d) => data.history.runs_by_day[d].unrunnable);

        statusChart = new Chart(ctx, {
            type: "bar",
            data: {
                labels,
                datasets: [
                    { label: "Successful", data: successful, backgroundColor: c.good, stack: "s", borderRadius: 4, maxBarThickness: 14 },
                    { label: "Failed", data: failed, backgroundColor: c.failed, stack: "s", borderRadius: 4, maxBarThickness: 14 },
                    { label: "Unrunnable", data: unrunnable, backgroundColor: c.unrunnable, stack: "s", borderRadius: 4, maxBarThickness: 14 },
                ],
            },
            options: {
                responsive: true,
                interaction: { mode: "index", intersect: false },
                plugins: {
                    legend: {
                        position: "top",
                        align: "start",
                        labels: { color: c.text, boxWidth: 9, boxHeight: 9, usePointStyle: true, pointStyle: "circle", font: { size: 11 } },
                    },
                    tooltip: { backgroundColor: c.surface, titleColor: c.text, bodyColor: c.muted, borderColor: c.grid, borderWidth: 1, padding: 10 },
                },
                scales: {
                    x: { stacked: true, ticks: { color: c.muted, font: { size: 9 }, maxRotation: 0, autoSkip: true, maxTicksLimit: 6 }, grid: { display: false }, border: { color: c.grid } },
                    y: { stacked: true, beginAtZero: true, ticks: { color: c.muted, precision: 0, font: { size: 10 } }, grid: { color: c.grid }, border: { display: false } },
                },
            },
        });
    }

    function renderDurationChart() {
        const c = chartColors();
        const ctx = document.getElementById("chart-duration");
        if (!ctx) return;
        const bins = data.history.execution_durations.bins;
        const labels = bins.map((b) => `${formatMinSec(b.lower_secs)}–${formatMinSec(b.upper_secs)}`);
        const values = bins.map((b) => b.count);

        durationChart = new Chart(ctx, {
            type: "bar",
            data: {
                labels,
                datasets: [
                    {
                        label: "Executions",
                        data: values,
                        backgroundColor: c.accent,
                        borderRadius: { topLeft: 4, topRight: 4 },
                        borderSkipped: "bottom",
                        maxBarThickness: 18,
                        categoryPercentage: 0.9,
                        barPercentage: 0.9,
                    },
                ],
            },
            options: {
                responsive: true,
                plugins: {
                    legend: { display: false },
                    tooltip: {
                        backgroundColor: c.surface,
                        titleColor: c.text,
                        bodyColor: c.muted,
                        borderColor: c.grid,
                        borderWidth: 1,
                        padding: 10,
                        callbacks: { title: (items) => `${items[0].label} min` },
                    },
                },
                scales: {
                    x: { ticks: { color: c.muted, font: { size: 9 }, maxRotation: 0, autoSkip: true, maxTicksLimit: 6 }, grid: { display: false }, border: { color: c.grid } },
                    y: { beginAtZero: true, ticks: { color: c.muted, precision: 0, font: { size: 10 } }, grid: { color: c.grid }, border: { display: false } },
                },
            },
        });
    }

    function renderCharts() {
        if (statusChart) statusChart.destroy();
        if (durationChart) durationChart.destroy();
        renderStatusChart();
        renderDurationChart();
    }

    // Redraw with the new palette whenever the theme toggle flips data-theme.
    new MutationObserver(renderCharts).observe(document.documentElement, {
        attributes: true,
        attributeFilter: ["data-theme"],
    });

    renderCharts();
})();
