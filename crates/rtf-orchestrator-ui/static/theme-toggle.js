(function () {
    var stored = localStorage.getItem("rtf-theme");
    if (stored) document.documentElement.setAttribute("data-theme", stored);
})();

document.addEventListener("DOMContentLoaded", function () {
    var btn = document.getElementById("theme-toggle");
    if (!btn) return;

    function prefersDark() {
        return window.matchMedia("(prefers-color-scheme: dark)").matches;
    }

    function current() {
        var attr = document.documentElement.getAttribute("data-theme");
        if (attr) return attr;
        return prefersDark() ? "dark" : "light";
    }

    function render() {
        var isDark = current() === "dark";
        btn.textContent = isDark ? "☀️ Light" : "\u{1F319} Dark";
    }

    btn.addEventListener("click", function () {
        var next = current() === "dark" ? "light" : "dark";
        document.documentElement.setAttribute("data-theme", next);
        localStorage.setItem("rtf-theme", next);
        render();
    });

    render();
});
