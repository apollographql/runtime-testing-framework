// Renders the executions per hour chart on the index page.
//
// Data comes from the `#dashboard-data` script tag the template embeds - see
// `DashboardView::js_data` in src/view/dashboard.rs for exactly what's in it.
(function() {
    const dataEl = document.getElementById("dashboard-data");
    if (!dataEl) return;
    const data = JSON.parse(dataEl.textContent);

    function cssVar(name) {
        return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
    }

    function chartColors() {
        return {
            series: [
                cssVar("--color-brand-accent"),
                cssVar("--status-active-text"),
                cssVar("--status-success-text"),
                cssVar("--status-unrunnable-text"),
                cssVar("--status-failed-text"),
            ],
            muted: cssVar("--color-text-secondary"),
            grid: cssVar("--color-border-primary"),
            surface: cssVar("--color-bg-primary"),
            text: cssVar("--color-text-primary"),
        };
    }

    let executionsChart;

    function renderExecutionsChart() {
        const c = chartColors();
        const ctx = document.getElementById("chart-executions");
        if (!ctx) return;
        const labels = data.hours.map((h) =>
            new Date(h).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
        );
        const datasets = data.clusters.map((cluster, i) => {
            const color = c.series[i % c.series.length];
            return {
                label: cluster.name,
                data: cluster.counts,
                borderColor: color,
                backgroundColor: color,
                borderWidth: 2,
                pointRadius: 0,
                pointHoverRadius: 3,
                tension: 0.3,
            };
        });

        executionsChart = new Chart(ctx, {
            type: "line",
            data: { labels, datasets },
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
                    x: { ticks: { color: c.muted, font: { size: 9 }, maxRotation: 0, autoSkip: true, maxTicksLimit: 6 }, grid: { display: false }, border: { color: c.grid } },
                    y: { beginAtZero: true, ticks: { color: c.muted, precision: 0, font: { size: 10 } }, grid: { color: c.grid }, border: { display: false } },
                },
            },
        });
    }

    function renderCharts() {
        if (executionsChart) executionsChart.destroy();
        renderExecutionsChart();
    }

    // Redraw with the new palette whenever the theme toggle flips data-theme.
    new MutationObserver(renderCharts).observe(document.documentElement, {
        attributes: true,
        attributeFilter: ["data-theme"],
    });

    renderCharts();
})();
