// The test plan detail page's client-side behaviour: the trigger form (an Alpine.js component whose
// markup lives in templates/test_plan_detail.html, built on the model in trigger-model.js) and the
// two history charts. Everything else on this page is plain server-rendered HTML/links needing no
// JS.
//
// Data comes from the `#test-plan-details-data` script tag the template embeds - see
// `TestPlanDetailsView::js_data` in src/view/test_plan_details.rs for exactly what's in it.
//
// Loaded as a module ahead of the deferred alpine.min.js, so the component is registered before
// Alpine starts.
import { clone, coerce, createModel, fmt, parseOverrides, short } from "./trigger-model.js";

const dataEl = document.getElementById("test-plan-details-data");
if (dataEl) init(JSON.parse(dataEl.textContent));

function init(data) {
    const {
        VARS, PLAN_GROUPS, PLAN_GROUP_MEMBERS, freshState, defaults, isRequired, groupOf, standaloneVars, valuesOf,
        isChanged, countOf, formulaOf, variantsOf, problemsOf, groupChanged, toOverrides, fromOverrides,
        addToGroup, removeFromGroup, dissolveGroup, createGroup, resetGroup,
    } = createModel(data);
    const EMPTY_GROUP = { vars: [], entries: [] };

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
                isRequired(n) { return isRequired(n); },
                sectionsOf(n) { return VARS[n] ? VARS[n].sections : []; },
                descriptionOf(n) { return VARS[n] ? VARS[n].description : ""; },
                allowedOf(n) { return VARS[n] ? VARS[n].allowed : null; },
                hasValue(n, v) { return this.valuesFor(n).some((x) => x === v); },
                invalidValues(n) { return this.valuesFor(n).filter((x) => !this.allowedOf(n).includes(x)); },
                defaultTitle(n) { return (defaults(n) || []).map(fmt).join(", "); },
                defaultLabel(n) {
                    if (!VARS[n]) return this.isFreed(n) ? "from a compound dimension" : "not declared at this ref";
                    if (isRequired(n)) return "required: no default";
                    const def = defaults(n);
                    return def.length ? `default: ${def.map(short).join(", ")}` : "not set by default";
                },
                revertTitle(n) {
                    if (this.isFreed(n)) return "Needs a value: it has no default outside its compound dimension";
                    if (isRequired(n)) return "Required: this test plan has no default for it";
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
                    if (isRequired(n)) return;
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
                groupChanged(g) { return groupChanged(this, g); },
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
}
