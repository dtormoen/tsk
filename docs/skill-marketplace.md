# TSK Skills Marketplace

This repository serves as a Claude Code skills marketplace, providing installable skills that teach Claude how to work with TSK. Skills follow the [Agent Skills](https://agentskills.io) open standard.

## Installing Skills

### Add the marketplace

In Claude Code, add the TSK repository as a plugin marketplace:

```
/plugin marketplace add dtormoen/tsk
```

### Install a skill

Browse and install individual skills:

```
/plugin install tsk-help@dtormoen/tsk
```

### Manual installation

You can also copy skills directly. Replace `tsk-help` with any skill name from the Available Skills table below:

```bash
# User-level (available in all projects)
cp -r skills/tsk-help/skills/tsk-help ~/.claude/skills/

# Project-level (available only in this project)
cp -r skills/tsk-help/skills/tsk-help .claude/skills/
```

## Available Skills

| Skill | Description |
|-------|-------------|
| `tsk-help` | Teaches Claude core TSK commands for delegating tasks to AI agents |
| `tsk-docker-config` | Guides users through configuring TSK Docker container images |
| `tsk-add` | Queues a development task from the current conversation using tsk add |

## Contributing a New Skill

### Directory structure

Each skill is a plugin directory under `skills/`:

```
skills/
└── your-skill-name/
    ├── .claude-plugin/
    │   └── plugin.json          # Plugin metadata
    └── skills/
        └── your-skill-name/
            └── SKILL.md         # Skill content (Agent Skills format)
```

### plugin.json

Minimal metadata for the plugin:

```json
{
  "name": "your-skill-name",
  "description": "Brief description of what this skill does.",
  "author": {
    "name": "Your Name"
  }
}
```

### SKILL.md

The skill file uses YAML frontmatter followed by markdown content:

```markdown
---
name: your-skill-name
description: When to activate this skill (max 1024 chars).
---

# Your Skill Title

Markdown content teaching Claude the skill...
```

### Naming rules

- Max 64 characters
- Lowercase letters, numbers, and hyphens only
- Cannot contain "anthropic" or "claude"
- Description must be non-empty, max 1024 characters

### Submitting

1. Create your skill directory under `skills/`
2. Include `plugin.json` and `SKILL.md` as shown above
3. Add your plugin to `.claude-plugin/marketplace.json` in the `plugins` array
4. Open a pull request
