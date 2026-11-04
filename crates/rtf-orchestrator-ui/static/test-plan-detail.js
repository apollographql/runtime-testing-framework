// The test plan detail page's client-side behaviour: the trigger form (variable overrides, matrix
// and compound dimension editing, the generated variables JSON and the execution count shown in
// the trigger bar) and the two history charts. Everything else on this page is plain
// server-rendered HTML/links needing no JS.
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
(function() {
    const dataEl = document.getElementById("test-plan-details-data");
    if (!dataEl) return;
    const data = JSON.parse(dataEl.textContent);
    const $ = (id) => document.getElementById(id);

    function cssVar(name) {
        return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
    }

    // ==========================================================================
    // Override model
    // ==========================================================================

    const VARS = Object.fromEntries(data.variables.map((v) => [v.name, v]));
    const PLAN_GROUPS = Object.fromEntries(Object.entries(data.compound_groups).map(([name, entries]) => [
        name,
        { vars: entries.length ? Object.keys(entries[0]) : [], entries },
    ]));
    const PLAN_GROUP_MEMBERS = new Set(Object.values(PLAN_GROUPS).flatMap((g) => g.vars));

    const clone = (o) => JSON.parse(JSON.stringify(o));
    const fmt = (v) => (typeof v === "string" ? v : JSON.stringify(v));
    const canon = (o) => {
        if (Array.isArray(o)) return o.map(canon);
        if (o && typeof o === "object") return Object.fromEntries(Object.keys(o).sort().map((k) => [k, canon(o[k])]));
        return o;
    };
    const sameJSON = (a, b) => JSON.stringify(canon(a)) === JSON.stringify(canon(b));

    function coerce(s) {
        s = String(s).trim();
        if (s === "true") return true;
        if (s === "false") return false;
        if (/^-?\d+(\.\d+)?$/.test(s)) return Number(s);
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
    const valuesOf = (s, n) => (s.selected.includes(n) ? s.values[n] : defaults(n) || []);

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
    // DOM helpers
    // ==========================================================================

    function h(tag, attrs, ...kids) {
        const el = document.createElement(tag);
        for (const [k, v] of Object.entries(attrs || {})) {
            if (v == null || v === false) continue;
            if (k.startsWith("on")) el.addEventListener(k.slice(2).toLowerCase(), v);
            else if (k === "class") el.className = v;
            else if (k === "value") el.value = v;
            else el.setAttribute(k, v === true ? "" : v);
        }
        for (const c of kids.flat()) {
            if (c != null && c !== false) el.append(c instanceof Node ? c : document.createTextNode(String(c)));
        }
        return el;
    }

    let focusAfter = null;
    function restoreFocus() {
        if (!focusAfter) return;
        const el = document.querySelector(`[data-focus="${CSS.escape(focusAfter)}"]`);
        focusAfter = null;
        if (el) el.focus();
    }

    let toastTimer;
    function toast(msg) {
        const t = $("toast");
        t.textContent = msg;
        t.classList.add("show");
        clearTimeout(toastTimer);
        toastTimer = setTimeout(() => t.classList.remove("show"), 3500);
    }

    function short(v) {
        const s = fmt(v);
        const first = s.split("\n")[0];
        return first.length > 40 || s.includes("\n") ? `${first.slice(0, 40)}…` : s;
    }

    const roleBadge = (vals) => (vals.length > 1
        ? h("span", { class: "pill pill--matrix" }, `matrix ×${vals.length}`)
        : h("span", { class: "pill pill--fixed" }, "fixed"));

    function eyeIcon() {
        const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
        svg.setAttribute("viewBox", "0 0 16 16");
        svg.setAttribute("width", "14");
        svg.setAttribute("height", "14");
        svg.innerHTML = '<path d="M1 8s2.5-4.5 7-4.5S15 8 15 8s-2.5 4.5-7 4.5S1 8 1 8z" fill="none" stroke="currentColor" stroke-width="1.3"></path>'
            + '<circle cx="8" cy="8" r="2" fill="none" stroke="currentColor" stroke-width="1.3"></circle>';
        return svg;
    }

    function fullValueButton(n, vals) {
        const id = `value-popover-${n}`;
        return [
            h("button", { type: "button", class: "copy-btn", popovertarget: id, title: "View and copy full value", "aria-label": `View full value of ${n}` }, eyeIcon()),
            h("div", { id, popover: true },
                h("p", { class: "muted", style: "margin-top:0" }, "Current value of ", h("code", {}, n)),
                h("pre", {}, h("code", {}, vals.map(fmt).join("\n"))),
                h("p", {}, h("button", { type: "button", class: "secondary", popovertarget: id, popovertargetaction: "hide" }, "Close"))),
        ];
    }

    // Values as chips: one chip = fixed value, several = matrix dimension. Variables with allowed
    // values get toggle buttons instead.
    function valueEditor(name, values, set) {
        const v = VARS[name];
        if (v && v.allowed) {
            return h("div", { class: "toggles" },
                v.allowed.map((opt) => {
                    const on = values.some((x) => x === opt);
                    return h("button", {
                        type: "button", class: "toggle-opt" + (on ? " on" : ""), "aria-pressed": String(on),
                        onclick: () => set(on ? values.filter((x) => x !== opt) : v.allowed.filter((a) => values.includes(a) || a === opt)),
                    }, fmt(opt));
                }),
                values.filter((x) => !v.allowed.includes(x)).map((bad) => h("span", { class: "chip chip--invalid", title: "Not an allowed value" },
                    h("span", { class: "chip__text" }, fmt(bad)),
                    h("button", { type: "button", class: "chip__remove", "aria-label": `Remove ${fmt(bad)}`, onclick: () => set(values.filter((x) => x !== bad)) }, "×"))));
        }

        const wrap = h("div", { class: "chip-input" });
        const input = h("input", {
            "data-focus": `add:${name}`,
            placeholder: values.length ? "+ value" : "type a value, Enter to add",
            "aria-label": `Add value for ${name}`,
            onkeydown: (e) => {
                if (e.key === "Enter" || e.key === ",") {
                    e.preventDefault();
                    commit(true);
                } else if (e.key === "Backspace" && !input.value && values.length) {
                    focusAfter = `add:${name}`;
                    set(values.slice(0, -1));
                }
            },
            onblur: () => commit(false),
        });

        function commit(refocus) {
            const parts = input.value.split(",").map((x) => x.trim()).filter(Boolean).map(coerce);
            input.value = "";
            if (!parts.length) return;
            if (refocus) focusAfter = `add:${name}`;
            set([...values, ...parts]);
        }

        function edit(chip, i) {
            let done = false;
            const multiline = typeof values[i] === "string" && values[i].includes("\n");
            const ed = h(multiline ? "textarea" : "input", {
                class: "chip-edit", value: fmt(values[i]), size: Math.max(6, fmt(values[i]).length + 2), rows: multiline ? 6 : null,
                onkeydown: (e) => {
                    if (e.key === "Enter" && (!multiline || e.metaKey || e.ctrlKey)) {
                        e.preventDefault();
                        ed.blur();
                    }
                    if (e.key === "Escape") {
                        done = true;
                        render();
                    }
                },
                onblur: () => {
                    if (done) return;
                    done = true;
                    const next = [...values];
                    const nv = multiline ? ed.value : ed.value.trim();
                    if (nv.trim() === "") next.splice(i, 1);
                    else next[i] = multiline ? nv : coerce(nv);
                    set(next);
                },
            });
            chip.replaceWith(ed);
            ed.focus();
            if (!multiline) ed.select();
        }

        values.forEach((val, i) => {
            const chip = h("span", { class: "chip" },
                h("span", { class: "chip__text", title: "Click to edit", onclick: () => edit(chip, i) }, short(val)),
                h("button", { type: "button", class: "chip__remove", "aria-label": `Remove ${fmt(val)}`, onclick: () => set(values.filter((_, j) => j !== i)) }, "×"));
            wrap.append(chip);
        });
        wrap.addEventListener("mousedown", (e) => {
            if (e.target === wrap) {
                e.preventDefault();
                input.focus();
            }
        });
        wrap.append(input);
        return wrap;
    }

    // ==========================================================================
    // Rendering
    // ==========================================================================

    let state = freshState();
    let newGroupOpen = false;
    // Set while the raw JSON textarea holds content that hasn't been applied to the form (invalid
    // JSON from a rejected trigger, or hand edits): it is left alone and triggering is blocked.
    let rawPending = false;

    function setValues(n, vals) {
        state.values[n] = vals;
        render();
    }

    function revert(n) {
        state.selected = state.selected.filter((x) => x !== n);
        delete state.values[n];
        render();
    }

    function renderOverrides() {
        const rows = state.selected.filter((n) => !groupOf(state, n));
        const nChanged = rows.filter((n) => isChanged(state, n)).length;
        $("changed-count").textContent = rows.length ? `${nChanged} changed from default` : "";
        if (!rows.length) {
            $("overrides").replaceChildren(h("div", { class: "empty-state muted" }, "Nothing changed — every variable runs at its test plan default."));
            return;
        }
        $("overrides").replaceChildren(...rows.map((n) => {
            const v = VARS[n];
            const def = defaults(n);
            const freed = !v && PLAN_GROUP_MEMBERS.has(n);
            const defaultLabel = !v
                ? (freed ? "from a compound dimension" : "not declared at this ref")
                : def.length ? `default: ${def.map(short).join(", ")}` : "not set by default";
            return h("div", { class: "override-row" },
                h("div", { class: "override-row__name" },
                    h("code", {}, n), " ",
                    v ? v.sections.map((s) => h("span", { class: "tag" }, s)) : "",
                    v && v.description ? h("div", { class: "muted" }, v.description) : ""),
                valueEditor(n, state.values[n], (nv) => setValues(n, nv)),
                h("div", { class: "override-row__meta" },
                    roleBadge(state.values[n]),
                    h("span", { class: "muted", title: def ? def.map(fmt).join(", ") : "" }, defaultLabel),
                    h("button", {
                        type: "button", class: "icon", disabled: freed,
                        title: freed ? "Needs a value: it has no default outside its compound dimension" : v ? "Revert to default" : "Remove",
                        "aria-label": `Revert ${n}`, onclick: () => revert(n),
                    }, v ? "↺" : "×")));
        }));
    }

    function compoundTable(g, d) {
        const addSelect = h("select", {
            class: "small", "aria-label": `Add a variable to ${g}`,
            onchange: (e) => {
                const n = e.target.value;
                if (!n) return;
                const k = addToGroup(state, g, n);
                if (k > 1) toast(`${n} had ${k} values: entries were multiplied out so the same executions still run`);
                render();
            },
        }, h("option", { value: "" }, "+ variable"),
        standaloneVars(state).map((n) => h("option", { value: n }, `${n} = ${valuesOf(state, n).map(short).join(", ") || "(not set)"}`)));

        const header = h("tr", {},
            h("th", {}, "#"),
            d.vars.map((n) => h("th", {}, h("code", {}, n), " ",
                h("button", {
                    type: "button", class: "icon", title: `Remove ${n} from ${g} (it becomes a fixed value)`, "aria-label": `Remove ${n} from ${g}`,
                    onclick: () => {
                        const v = removeFromGroup(state, g, n);
                        toast(`${n} is now fixed at ${fmt(v[0])}`);
                        render();
                    },
                }, "×"))),
            h("th", {}, addSelect));

        const body = d.entries.map((e, i) => h("tr", {},
            h("td", { class: "idx" }, i + 1),
            d.vars.map((n) => h("td", {}, h("input", {
                value: fmt(e[n] ?? ""), "aria-label": `${g} entry ${i + 1} ${n}`,
                oninput: (ev) => {
                    e[n] = coerce(ev.target.value);
                    renderDerived();
                },
            }))),
            h("td", { class: "actions" },
                h("button", { type: "button", class: "icon", title: "Duplicate entry", "aria-label": `Duplicate entry ${i + 1}`, onclick: () => { d.entries.splice(i + 1, 0, clone(e)); render(); } }, "⧉"),
                h("button", { type: "button", class: "icon text-danger", title: "Delete entry", "aria-label": `Delete entry ${i + 1}`, onclick: () => { d.entries.splice(i, 1); render(); } }, "×"))));

        return h("table", { class: "compound-table" }, h("thead", {}, header), h("tbody", {}, body));
    }

    function renderGroups() {
        const deleted = Object.keys(PLAN_GROUPS).filter((g) => !state.groups[g]);
        const cards = Object.entries(state.groups).map(([g, d]) => {
            const plan = PLAN_GROUPS[g];
            const changed = !plan || !sameJSON(plan.entries, groupEntries(d));
            return h("div", { class: "compound-group" + (changed ? " changed" : "") },
                h("div", { class: "compound-group__head" },
                    h("code", { style: "font-weight:600" }, g),
                    h("span", { class: "pill pill--matrix" }, `${d.entries.length} entries`),
                    h("span", { class: "muted" }, plan ? (changed ? "from test plan · modified" : "from test plan") : "new"),
                    h("span", { class: "spacer" }),
                    plan && changed ? h("button", { type: "button", class: "small", onclick: () => { resetGroup(state, g); render(); } }, "Reset") : "",
                    h("button", {
                        type: "button", class: "small text-danger", title: "Delete this dimension; its variables become fixed values",
                        onclick: () => {
                            dissolveGroup(state, g);
                            toast(`${g} deleted — its variables are now fixed values you can edit above`);
                            render();
                        },
                    }, "Delete dimension")),
                h("div", { class: "compound-group__body" },
                    compoundTable(g, d),
                    h("p", { style: "margin:0.5rem 0 0" }, h("button", {
                        type: "button", class: "small",
                        onclick: () => {
                            d.entries.push(d.entries.length ? clone(d.entries[d.entries.length - 1]) : Object.fromEntries(d.vars.map((v) => [v, ""])));
                            render();
                        },
                    }, "+ Add entry"))));
        });
        if (deleted.length) {
            cards.push(h("p", { class: "muted" }, `Deleted for this run: ${deleted.join(", ")} · `,
                deleted.map((g) => h("button", { type: "button", class: "link", onclick: () => { resetGroup(state, g); render(); } }, `restore ${g}`))));
        }
        if (!cards.length) cards.push(h("div", { class: "empty-state muted" }, "No compound dimensions."));
        $("groups").replaceChildren(...cards);
    }

    function renderNewGroup() {
        $("new-group-btn").hidden = newGroupOpen;
        if (!newGroupOpen) {
            $("new-group-slot").replaceChildren();
            return;
        }
        const nameInput = h("input", { type: "text", placeholder: "dimension_name", "data-focus": "new-group-name", "aria-label": "New compound dimension name" });
        const checks = standaloneVars(state).map((n) => h("label", {},
            h("input", { type: "checkbox", value: n }),
            h("code", {}, n),
            h("span", { class: "muted" }, valuesOf(state, n).map(short).join(", "))));
        const create = () => {
            const name = nameInput.value.trim().replace(/\s+/g, "_");
            const picked = checks.map((c) => c.querySelector("input")).filter((i) => i.checked).map((i) => i.value);
            if (!name) return toast("Give the dimension a name");
            if (state.groups[name] || VARS[name] || PLAN_GROUPS[name]) return toast(`${name} is already in use`);
            if (!picked.length) return toast("Pick at least one variable");
            createGroup(state, name, picked);
            newGroupOpen = false;
            render();
        };
        $("new-group-slot").replaceChildren(h("div", { class: "new-group" },
            h("strong", {}, "New compound dimension"),
            h("p", { class: "muted", style: "margin:0.25rem 0" }, "Pick the variables that should vary together. Entries start as every combination of their current values — delete the rows you don't want."),
            h("p", {}, nameInput),
            h("div", { class: "new-group__checks" }, checks),
            h("button", { type: "button", class: "primary", onclick: create }, "Create"), " ",
            h("button", { type: "button", class: "secondary", onclick: () => { newGroupOpen = false; render(); } }, "Cancel")));
    }

    function renderDefaults() {
        const rows = standaloneVars(state).filter((n) => !state.selected.includes(n));
        $("defaults-count").textContent = `(${rows.length})`;
        $("defaults-body").replaceChildren(...rows.map((n) => {
            const v = VARS[n];
            return h("tr", {},
                h("td", {}, h("code", {}, n), " ", v.sections.map((s) => h("span", { class: "tag" }, s))),
                h("td", {},
                    v.default.length ? h("code", {}, v.default.map(short).join(", ")) : h("span", { class: "muted" }, "not set"), " ",
                    v.default.length > 1 ? roleBadge(v.default) : "",
                    v.default.length ? fullValueButton(n, v.default) : ""),
                h("td", { class: "muted" }, v.description),
                h("td", {}, h("button", { type: "button", class: "small", onclick: () => selectVar(n) }, "Change")));
        }));
    }

    function renderDerived() {
        const total = countOf(state);
        const problems = problemsOf(state);
        if (rawPending) problems.unshift("apply the raw variables JSON to the form first");
        const btn = $("trigger-btn");
        const forbidden = "forbidden" in btn.dataset;

        $("trigger-total").textContent = total.toLocaleString();
        $("trigger-count").textContent = total.toLocaleString();
        $("trigger-formula").textContent = formulaOf(state);
        $("trigger-problems").textContent = problems.length ? `⚠ ${problems.join("; ")}` : "";
        btn.disabled = forbidden || total === 0 || problems.length > 0;

        const textarea = $("variables");
        if (!rawPending && document.activeElement !== textarea) {
            const overrides = toOverrides(state);
            textarea.value = Object.keys(overrides).length ? JSON.stringify(overrides, null, 2) : "";
        }
    }

    function render() {
        renderOverrides();
        renderGroups();
        renderNewGroup();
        renderDefaults();
        renderDerived();
        renderPicker();
        restoreFocus();
    }

    // --- variable picker ---
    function renderPicker() {
        const q = $("picker-q").value.toLowerCase();
        const items = data.variables.filter((v) => !state.selected.includes(v.name)
            && (v.name.toLowerCase().includes(q) || v.description.toLowerCase().includes(q)));
        if (!items.length) {
            $("picker-list").replaceChildren(h("li", { class: "muted", style: "padding:6px" }, "No matching variables"));
            return;
        }
        $("picker-list").replaceChildren(...items.map((v, i) => h("li", {},
            h("button", { type: "button", class: i === 0 && q ? "active" : "", onclick: () => selectVar(v.name) },
                h("span", { class: "picker__row" },
                    h("span", {}, h("code", { style: "font-weight:600" }, v.name), " ", v.sections.map((s) => h("span", { class: "tag" }, s))),
                    h("code", { class: "muted" }, v.default.map(short).join(", "))),
                v.description ? h("span", { class: "muted", style: "display:block" }, v.description) : ""))));
    }

    function selectVar(n) {
        if (!state.selected.includes(n)) {
            state.selected.push(n);
            state.values[n] = clone(defaults(n) || []);
        }
        $("picker").hidden = true;
        $("picker-q").value = "";
        focusAfter = `add:${n}`;
        render();
    }

    $("add-var").addEventListener("click", () => {
        $("picker").hidden = !$("picker").hidden;
        if (!$("picker").hidden) {
            renderPicker();
            $("picker-q").focus();
        }
    });
    $("picker-q").addEventListener("input", renderPicker);
    $("picker-q").addEventListener("keydown", (e) => {
        if (e.key === "Escape") $("picker").hidden = true;
        if (e.key === "Enter") {
            e.preventDefault();
            const b = $("picker-list").querySelector("button");
            if (b) b.click();
        }
    });
    document.addEventListener("mousedown", (e) => {
        if (!e.target.closest(".picker-wrap")) $("picker").hidden = true;
    });

    // --- raw JSON ---
    function parseOverrides(raw) {
        const obj = raw.trim() ? JSON.parse(raw) : {};
        if (typeof obj !== "object" || Array.isArray(obj) || obj === null) throw new Error("must be a JSON object");
        return obj;
    }

    $("apply-json").addEventListener("click", () => {
        try {
            state = fromOverrides(parseOverrides($("variables").value));
            rawPending = false;
            $("json-error").textContent = "";
            $("load-banner").replaceChildren();
            render();
            toast("Form populated from JSON");
        } catch (err) {
            $("json-error").textContent = `Invalid JSON: ${err.message}`;
        }
    });
    $("variables").addEventListener("input", () => {
        rawPending = true;
        renderDerived();
    });

    // --- the rest of the trigger bar ---
    $("new-group-btn").addEventListener("click", () => {
        newGroupOpen = true;
        focusAfter = "new-group-name";
        render();
    });
    $("variants-btn").addEventListener("click", () => {
        const { cols, rows } = variantsOf(state);
        const total = countOf(state);
        $("variants-title").textContent = `${total.toLocaleString()} executions`;
        $("variants-sub").textContent = `${formulaOf(state)}. Only varying variables are shown${rows.length < total ? `; first ${rows.length} listed` : ""}.`;
        $("variants-table").replaceChildren(
            h("thead", {}, h("tr", {}, h("th", {}, "#"), cols.map((c) => h("th", {}, c)))),
            h("tbody", {}, rows.map((r, i) => h("tr", {}, h("td", { class: "muted" }, i + 1), cols.map((c) => h("td", {}, h("code", {}, short(r[c]))))))));
        $("variants-dialog").showModal();
    });
    document.querySelectorAll("[data-close-dialog]").forEach((b) => b.addEventListener("click", () => b.closest("dialog").close()));

    // Variables rendered into the textarea by the server (a "Re-run" link, or a rejected trigger)
    // seed the form. Anything that can't be parsed is left untouched for the user to fix.
    function seedFromInitialVariables() {
        const raw = $("variables").value;
        if (!raw.trim()) return;
        try {
            state = fromOverrides(parseOverrides(raw));
        } catch (err) {
            rawPending = true;
            $("raw-variables").open = true;
            $("json-error").textContent = `Invalid JSON: ${err.message}`;
            $("load-banner").replaceChildren(h("p", { class: "warn-box", style: "margin:0.75rem 0 0" },
                "The submitted variables couldn't be loaded into the form. They're kept unchanged in ",
                h("strong", {}, "Raw variables JSON"), " below."));
        }
    }

    // ==========================================================================
    // History charts
    // ==========================================================================

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

    if ($("trigger-section")) {
        seedFromInitialVariables();
        render();
    }
    renderCharts();
})();
