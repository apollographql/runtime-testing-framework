// Renders the executions per hour chart and the per-cluster node charts on the index page.
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

    // The categorical palette is --chart-0, --chart-1, ... in theme.css
    function palette() {
        const colors = [];
        for (let i = 0; ; i++) {
            const color = cssVar("--chart-" + i);
            if (!color) return colors;
            colors.push(color);
        }
    }

    function chartColors() {
        return {
            series: palette(),
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

    let nodeCharts = [];

    function renderNodeCharts() {
        const c = chartColors();
        document.querySelectorAll("canvas.node-chart").forEach((canvas) => {
            const cluster = data.clusters.find((cl) => cl.name === canvas.dataset.cluster);
            if (!cluster || cluster.instance_types.length === 0) return;

            nodeCharts.push(new Chart(canvas, {
                type: "doughnut",
                data: {
                    labels: cluster.instance_types.map((t) => t.name),
                    datasets: [{
                        data: cluster.instance_types.map((t) => t.count),
                        backgroundColor: cluster.instance_types.map((t) => c.series[t.colour]),
                        borderColor: c.surface,
                        borderWidth: 2,
                    }],
                },
                options: {
                    responsive: false,
                    animation: false,
                    cutout: "55%",
                    plugins: {
                        legend: { display: false },
                        tooltip: { backgroundColor: c.surface, titleColor: c.text, bodyColor: c.muted, borderColor: c.grid, borderWidth: 1, padding: 8 },
                    },
                },
            }));
        });
    }

    function renderCharts() {
        if (executionsChart) executionsChart.destroy();
        nodeCharts.forEach((chart) => chart.destroy());
        nodeCharts = [];
        renderExecutionsChart();
        renderNodeCharts();
    }

    // Redraw with the new palette whenever the theme toggle flips data-theme.
    new MutationObserver(renderCharts).observe(document.documentElement, {
        attributes: true,
        attributeFilter: ["data-theme"],
    });

    renderCharts();
})();
