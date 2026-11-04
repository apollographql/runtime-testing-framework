// The test plan page's trigger form model: a set of variable overrides on top of the test plan's
// defaults, and the conversion to and from the `VariableOverride` JSON the trigger endpoint
// expects. Kept free of DOM and Alpine code so it can be unit tested with `node --test` (see
// tests/js/).
//
// The state is:
// - `selected`/`values`: standalone variables the user has chosen to change, and their values
//   (one value = fixed, several = a matrix dimension).
// - `groups`: the effective compound dimensions, starting as a copy of the test plan's.

export const clone = (o) => JSON.parse(JSON.stringify(o));
export const fmt = (v) => (typeof v === "string" ? v : JSON.stringify(v));

const canon = (o) => {
    if (Array.isArray(o)) return o.map(canon);
    if (o && typeof o === "object") return Object.fromEntries(Object.keys(o).sort().map((k) => [k, canon(o[k])]));
    return o;
};
export const sameJSON = (a, b) => JSON.stringify(canon(a)) === JSON.stringify(canon(b));

export function short(v) {
    const s = fmt(v);
    const first = s.split("\n")[0];
    return first.length > 40 || s.includes("\n") ? `${first.slice(0, 40)}…` : s;
}

// Only text that round-trips exactly becomes a number, so e.g. "1.30" or "007" stay strings and
// an input's displayed text never changes under the cursor while typing.
export function coerce(s) {
    s = String(s).trim();
    if (s === "true") return true;
    if (s === "false") return false;
    if (/^-?\d+(\.\d+)?$/.test(s) && String(Number(s)) === s) return Number(s);
    return s;
}

export function parseOverrides(raw) {
    const obj = raw.trim() ? JSON.parse(raw) : {};
    if (typeof obj !== "object" || Array.isArray(obj) || obj === null) throw new Error("must be a JSON object");
    return obj;
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

// `data` is the page data embedded by the server: see `TestPlanDetailsView::js_data`.
export function createModel(data) {
    const VARS = Object.fromEntries(data.variables.map((v) => [v.name, v]));
    const PLAN_GROUPS = Object.fromEntries(Object.entries(data.compound_groups).map(([name, entries]) => [
        name,
        { vars: entries.length ? Object.keys(entries[0]) : [], entries },
    ]));
    const PLAN_GROUP_MEMBERS = new Set(Object.values(PLAN_GROUPS).flatMap((g) => g.vars));

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

    const groupChanged = (s, g) =>
        !s.groups[g] || !PLAN_GROUPS[g] || !sameJSON(PLAN_GROUPS[g].entries, groupEntries(s.groups[g]));

    function toOverrides(s) {
        const out = {};
        for (const n of s.selected) {
            if (groupOf(s, n) || !isChanged(s, n)) continue;
            const vals = s.values[n];
            out[n] = vals.length === 1 ? vals[0] : vals;
        }
        for (const g of Object.keys(PLAN_GROUPS)) if (!s.groups[g]) out[g] = [];
        for (const [g, d] of Object.entries(s.groups)) {
            if (!groupChanged(s, g)) continue;
            out[g] = groupEntries(d);
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
    // until the user deletes the rows they don't want. Returns how many values it had.
    function addToGroup(s, g, n) {
        const vals = valuesOf(s, n).slice();
        const d = s.groups[g];
        d.entries = d.entries.flatMap((e) => (vals.length ? vals : [""]).map((v) => ({ ...e, [n]: v })));
        d.vars.push(n);
        s.selected = s.selected.filter((x) => x !== n);
        delete s.values[n];
        return vals.length;
    }

    // A variable removed from a group becomes fixed at its first entry's value, which is returned.
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

    return {
        VARS,
        PLAN_GROUPS,
        PLAN_GROUP_MEMBERS,
        freshState,
        defaults,
        groupOf,
        standaloneVars,
        valuesOf,
        isChanged,
        countOf,
        formulaOf,
        variantsOf,
        problemsOf,
        groupChanged,
        toOverrides,
        fromOverrides,
        addToGroup,
        removeFromGroup,
        dissolveGroup,
        createGroup,
        resetGroup,
    };
}
