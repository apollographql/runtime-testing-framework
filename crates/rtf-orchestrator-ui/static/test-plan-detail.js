// The test plan detail page's client-side behaviour: the live executions-count preview as the
// trigger form's variable fields are edited, keeping the raw variables JSON textarea in sync with
// those fields, and rendering the two history charts. Everything else on this page (the "view full
// value" popovers, the ref-preview/paging links) is plain server-rendered HTML/links needing no JS.
//
// Data comes from the `#test-plan-details-data` script tag the template embeds - see
// `TestPlanDetailsView::js_data` in src/view.rs for exactly what's in it.
(function() {
    const dataEl = document.getElementById("test-plan-details-data");
    if (!dataEl) return;
    const data = JSON.parse(dataEl.textContent);

    function cssVar(name) {
        return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
    }

    function fieldFor(variableName) {
        return document.querySelector(`[data-var-name="${CSS.escape(variableName)}"]`);
    }

    function coerceValue(s) {
        return /^-?\d+(\.\d+)?$/.test(s) ? Number(s) : s;
    }

    function sameValues(a, b) {
        if (a.length !== b.length) return false;
        const bSet = new Set(b);
        return a.every((v) => bSet.has(v));
    }

    function readVariableState(variable, el) {
        if (!el) return { hasOverride: false, count: Math.max(variable.full_values.length, 1) };

        if (el.tagName === "SELECT") {
            const selected = Array.from(el.selectedOptions).map((o) => o.value);
            if (selected.length === 0 || sameValues(selected, variable.full_values)) {
                return { hasOverride: false, count: Math.max(variable.full_values.length, 1) };
            }
            return {
                hasOverride: true,
                count: selected.length,
                value: selected.length === 1 ? selected[0] : selected,
            };
        }

        const raw = el.value.trim();
        if (raw === "") {
            return { hasOverride: false, count: Math.max(variable.full_values.length, 1) };
        }
        const parts = raw.split(",").map((s) => s.trim()).filter((s) => s !== "");
        const coerced = parts.map(coerceValue);
        return {
            hasOverride: true,
            count: Math.max(parts.length, 1),
            value: coerced.length === 1 ? coerced[0] : coerced,
        };
    }

    // Keeps the raw JSON textarea (the actual field submitted to `/trigger`) in sync with whatever
    // the structured fields currently say - a bare value or comma-list becomes a scalar or array
    // override exactly as `VariableOverride` expects. Compound dimension overrides aren't
    // representable by these fields at all - that's what the raw JSON box itself is for.
    function syncVariablesJson() {
        const overrides = {};
        data.variables.forEach((variable) => {
            const state = readVariableState(variable, fieldFor(variable.name));
            if (state.hasOverride) overrides[variable.name] = state.value;
        });
        const textarea = document.getElementById("variables");
        if (textarea) {
            textarea.value = Object.keys(overrides).length ? JSON.stringify(overrides, null, 2) : "";
        }
    }

    function recompute() {
        let total = 1;
        const parts = [];

        data.variables.forEach((variable) => {
            const { count } = readVariableState(variable, fieldFor(variable.name));
            total *= count;
            if (count > 1) parts.push(`${count} ${variable.name}`);
        });
        data.compound_groups.forEach(([name, count]) => {
            total *= Math.max(count, 1);
            parts.push(`${count} ${name}`);
        });

        const statEl = document.getElementById("stat-executions");
        const formulaEl = document.getElementById("stat-formula");
        const triggerCountEl = document.getElementById("trigger-count");
        const triggerBtn = document.getElementById("trigger-btn");

        statEl.textContent = total.toLocaleString();
        statEl.classList.toggle("warn", total === 0);
        triggerCountEl.textContent = total.toLocaleString();
        triggerBtn.disabled = total === 0;
        formulaEl.textContent =
            total === 0
                ? "A field with no values selected produces no executions"
                : parts.join(" × ") || "1 execution";

        syncVariablesJson();
    }

    // Populates the structured fields from the raw variables JSON the server rendered into the
    // textarea (set when repopulating the form after a rejected trigger, or via the run page's
    // "Re-run" link) - `recompute()` reads the fields back out, so this must run before it.
    function seedFieldsFromInitialVariables() {
        const textarea = document.getElementById("variables");
        if (!textarea || !textarea.value.trim()) return;

        let overrides;
        try {
            overrides = JSON.parse(textarea.value);
        } catch {
            return; // invalid JSON can't be decomposed into fields; leave it for the raw textarea
        }
        if (typeof overrides !== "object" || overrides === null || Array.isArray(overrides)) return;

        Object.entries(overrides).forEach(([name, value]) => {
            const el = fieldFor(name);
            if (!el) return;
            const values = (Array.isArray(value) ? value : [value]).map(String);

            if (el.tagName === "SELECT") {
                Array.from(el.options).forEach((o) => { o.selected = values.includes(o.value); });
            } else {
                el.value = values.join(", ");
            }
        });
    }

    function attachFieldListeners() {
        data.variables.forEach((variable) => {
            const el = fieldFor(variable.name);
            if (!el) return;
            el.addEventListener(el.tagName === "SELECT" ? "change" : "input", recompute);
        });
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

    seedFieldsFromInitialVariables();
    attachFieldListeners();
    recompute();
    renderCharts();
})();
