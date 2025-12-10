<!-- diataxis-type: reference -->

# RTF Documentation Style Guide

This style guide defines writing standards for RTF documentation. It applies to all contributors
(internal and external) writing or editing docs in `docs/src/`.

## Base Style Guide

RTF adopts the [Microsoft Writing Style Guide][0] as its foundation. This document covers
RTF-specific decisions and deviations only. For topics not covered here, defer to Microsoft.

Key Microsoft principles we follow:

- Write like you speak
- Use second person ("you")
- Use active voice and present tense
- Use contractions (it's, you'll, we're)
- Get to the point fast
- Use sentence-case capitalization

---

## Diataxis Framework

RTF documentation follows the [Diataxis framework][1], which defines four documentation types. Each
page declares its type in an HTML comment on the first line.

### Type Declaration Format

Every documentation page must include a type declaration comment:

```markdown
<!-- diataxis-type: tutorial -->

# Page title

Content starts here...
```

Valid values: `tutorial`, `howto`, `reference`, `explanation`

The HTML comment format ensures the type declaration doesn't render in the built documentation while
remaining easy for contributors and tooling to identify.

### The Four Types

| Type            | Purpose                           | Reader State | Style                      |
| --------------- | --------------------------------- | ------------ | -------------------------- |
| **Tutorial**    | Teach through hands-on experience | Learning     | Guiding, step-by-step      |
| **How-to**      | Solve a specific problem          | Working      | Direct, action-focused     |
| **Reference**   | Describe the machinery            | Looking up   | Neutral, comprehensive     |
| **Explanation** | Provide context and background    | Studying     | Conversational, reflective |

### Type-Specific Guidelines

#### Tutorials

- Guide the reader step-by-step to a working result
- Every step should produce visible output
- Minimize explanation (link to Explanation docs instead)
- Use "you" throughout
- Celebrate milestones ("You now have a working test plan!")

#### How-to Guides

- Assume competence; don't teach fundamentals
- Focus on the task, not the concepts
- Use imperative mood ("Add the provider", "Run the command")
- No digressions or background information
- Title format: "How to [verb] [noun]" or "[Verb]-ing [noun]"

#### Reference

- Be neutral and factual
- Use consistent structure across similar pages
- Include all parameters, options, and fields
- Provide examples without explanation
- Use third person or passive voice where appropriate

#### Explanation

- Provide context, rationale, and background
- "We" voice is permitted ("We designed RTF to...")
- Connect concepts to each other
- Include trade-offs and design decisions
- May express opinions and preferences

---

## Voice and Tone

### Person

| Doc Type    | Person                               | Example                            |
| ----------- | ------------------------------------ | ---------------------------------- |
| Tutorial    | Second ("you")                       | "You configure the environment..." |
| How-to      | Second ("you") / Imperative          | "Configure the environment..."     |
| Reference   | Third / Neutral                      | "The environment defines..."       |
| Explanation | First plural ("we") + Second ("you") | "We designed this because..."      |

### "We" Voice

The team voice ("we at RTF", "we recommend") is permitted **only in Explanation docs**. All other
doc types should use "you" or imperative mood.

**Allowed (Explanation):**

> We here at Runtime Readiness are big fans of the Unix Philosophy.

**Not allowed (Tutorial/How-to/Reference):**

> ~~We recommend using the `--dry-run` flag.~~ → Use the `--dry-run` flag.

### Formality

- Tutorials and How-to guides: Friendly, contractions encouraged
- Reference: More formal, fewer contractions
- Explanation: Conversational, personality allowed

---

## Formatting

### Code and Commands

**Inline code** - Use backticks for:

- Commands: `rtf run`
- Config keys: `environment.setup`
- File paths: `test-plans/example.yaml`
- Values: `true`, `false`
- Flags: `--dry-run`

**Code blocks** - Use triple backticks with language identifier:

````markdown
```yaml
environment:
  name: production
```
````

**Command examples** - Do NOT include shell prompts:

```markdown
<!-- Good -->

rtf run my-plan.yaml --environment staging.yaml

<!-- Bad -->

$ rtf run my-plan.yaml --environment staging.yaml
```

**Commands with output** - Use separate code blocks:

Only show output when it adds value (reader needs to copy or verify something). Use an introductory
phrase to separate the command from its output:

```markdown
Verify the test plan templates correctly:

    rtf template test-plan.yaml --check

The output is similar to this:

    name: Hello World
    description: A test plan created as a guide
    ...
```

Guidelines for output:

- Use "The output is similar to this:" or "Output:" as the intro phrase
- Use `...` on its own line to indicate omitted output
- Keep output concise; trim to the relevant lines

### Placeholders

Use angle brackets for user-supplied values:

```
rtf run <test-plan> --environment <env-file>
```

Explain placeholders if not self-evident:

> Where `<test-plan>` is the path to your test plan YAML file.

### Headings

- Use sentence case (capitalize first word only)
- No trailing punctuation
- Use H2 (`##`) for main sections, H3 (`###`) for subsections
- Avoid H1 (`#`) except for page title

| Good                      | Bad                       |
| ------------------------- | ------------------------- |
| Configure the environment | Configure The Environment |
| Running test plans        | Running Test Plans.       |

### Lists

- Use numbered lists for sequential steps
- Use bullet lists for non-sequential items
- Use the Oxford comma in inline lists ("setup, run, and teardown")

### Links

Use **reference-style links** with definitions at the bottom of the file:

```markdown
See the [Test Plan][0] reference and [Command Provider][1] docs.

<!-- at bottom of file -->

[0]: ./test-plans.md
[1]: ./command-providers.md
```

Guidelines:

- Use numbered references (`[0]`, `[1]`, etc.) for simplicity
- Place all link definitions at the bottom of the file
- Use relative paths for internal links
- Use descriptive link text, not "click here" or bare URLs

| Good                                 | Bad                                 |
| ------------------------------------ | ----------------------------------- |
| See the [Test Plans][0] reference.   | See [here][0].                      |
| Configure [environments][1].         | https://example.com/environments.md |
| The [glossary][2] defines this term. | Click [this link](./glossary.md).   |

---

## Terminology

### Glossary Usage

RTF maintains a central [glossary][2]. When using RTF-specific terms:

1. **Always** ensure the term is defined in the glossary
2. **Link to the glossary** on first use in a page
3. **Inline definitions are encouraged** in Tutorials where clicking away disrupts flow

Example (Tutorial):

> A **Test Plan** is the top-level entry point for RTF that defines variables and references to
> Scenarios and Environments. See the [glossary][2] for the full definition.

Example (How-to/Reference):

> Configure the [Test Plan][2] with your variables.

### RTF Concepts

Use these exact capitalizations:

| Term        | Usage                                                            |
| ----------- | ---------------------------------------------------------------- |
| Test Plan   | UpperCamelCase as concept; `test-plan.yaml` for files            |
| Environment | UpperCamelCase as concept                                        |
| Scenario    | UpperCamelCase as concept                                        |
| Provider    | Generic term; specific types are File Provider, Command Provider |
| RTF         | Always uppercase, no periods                                     |

### Config Keys

Use backticks and exact casing from the YAML schema:

- `variables`, `matrix`, `environment.setup`
- NOT: "Variables", "the matrix key", "`VARIABLES`"

---

## Inclusive Language

Follow [Microsoft's inclusive language guidelines][3]. Key points:

### Pronouns

- Use singular "they" for gender-neutral reference
- Prefer "you" to avoid pronouns entirely
- Never use "he" as generic

### Terms to Avoid

| Avoid               | Use Instead                      |
| ------------------- | -------------------------------- |
| blacklist/whitelist | denylist/allowlist               |
| master/slave        | primary/replica, leader/follower |
| sanity check        | confidence check, quick check    |
| dummy               | placeholder, sample              |
| simple/easy         | (use sparingly; subjective)      |

### Accessibility

- Don't use color alone to convey meaning
- Use descriptive link text

### Images

- Include images **only when necessary** (diagrams of complex flows, UI screenshots)
- Prefer text and code examples over images where possible
- **Always** provide meaningful alt text that conveys the image's purpose

```markdown
![Test plan execution flow showing setup, scenario, and teardown phases](./images/execution-flow.png)
```

---

## Document Structure

### Page Length

No fixed limit. Pages should cover one focused topic. If a page requires more than 2 heading levels
or you find yourself scrolling extensively, consider splitting into subpages.

Guidelines:

- Define scope clearly at the start (what the page covers and what it doesn't)
- Front-load key information; readers scan rather than read linearly
- Cut everything unnecessary; prefer a short, accurate page over a comprehensive stale one
- Structure for skimmability: short paragraphs, bullet points, tables

### Standard Sections

**Tutorials** should include:

1. Overview (what you'll learn/build)
2. Prerequisites
3. Step-by-step instructions
4. Next steps

**How-to guides** should include:

1. Brief intro (1-2 sentences)
2. Prerequisites (if any)
3. Steps
4. (Optional) Troubleshooting

**Reference** pages should include:

1. Brief description
2. All fields/options (consistent format)
3. Examples
4. Related pages

**Explanation** pages have flexible structure based on content.

### Prerequisites

List prerequisites in a blockquote or admonition:

```markdown
> **Prerequisites**
>
> - RTF installed (`cargo install rtf-cli`)
> - A GitHub access token exported as `GITHUB_TOKEN`
```

---

## Checklist for Contributors

Before submitting documentation:

- [ ] First line includes `<!-- diataxis-type: <type> -->` comment
- [ ] Voice matches doc type (no "we" in tutorials/how-to/reference)
- [ ] No shell prompts in command examples
- [ ] Placeholders use angle brackets
- [ ] RTF terminology capitalized correctly
- [ ] New terms added to glossary and linked on first use
- [ ] Links use reference-style with definitions at bottom
- [ ] Inclusive language guidelines followed

[0]: https://learn.microsoft.com/en-us/style-guide/welcome/
[1]: https://diataxis.fr/
[2]: ../reference/glossary.md
[3]: https://learn.microsoft.com/en-us/style-guide/bias-free-communication
