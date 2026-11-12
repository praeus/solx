# solx skills

Harness-agnostic skill definitions for working with solx. Each skill is a
directory containing a `SKILL.md` with YAML frontmatter (`name`, `description`)
plus optional `references/`, `scripts/` and `assets/`.

| Skill | What it is for |
| --- | --- |
| [`solx/`](solx/) | the solx catalogue: discovering actions with `explore_tools`, running them, and registering new ones |

## Wiring it into a harness

The layout here is the common denominator (Claude Code and OpenCode both read
`<dir>/<name>/SKILL.md` with `name` + `description` frontmatter), so a harness
either points at this directory or a copy of it is dropped into its own skills
folder.

**OpenCode** - add the parent directory to `skills.paths` in
`~/.config/opencode/opencode.json` (or a project `opencode.jsonc`):

```jsonc
{
  "$schema": "https://opencode.ai/config.json",
  "skills": {
    "paths": ["C:/Projects/solx/skills"]
  }
}
```

Newer OpenCode releases prefer a plain array, which older ones do not accept -
use whichever your version supports:

```jsonc
{ "skills": ["C:/Projects/solx/skills"] }
```

Alternatively copy `solx/` into `.opencode/skills/` (project) or
`~/.config/opencode/skills/` (global).

**Claude Code** - copy `solx/` into `.claude/skills/` (project) or
`~/.claude/skills/` (global).

**Anything else** - point its skill loader at this directory; each subdirectory
with a `SKILL.md` is one skill.

Run `opencode debug skill` to confirm a harness has picked the skills up.
