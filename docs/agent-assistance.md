# Agent Assistance

## Principles

> **Scope of this document** — this document defines the end-state work-collaboration capability goals of Agent Assistance; it does not define capability plans, generalization tasks, evaluation projects, coverage, or evolution — see `docs/capability-evaluation-project.md`; nor does it define the Case Runner infrastructure — see `docs/case-runner.md`.

## Agent Assistance

The Agent's fundamental role is a **work supervision and collaboration assistant**: around the user's ongoing multi-line work, understand facts, manage attention, and advance collaboration. Config, terminal, Context, Skill, visualization tools, jumps, and UI are all means to these ends, not capability categories themselves.

**Multi-line plan progress Track**: treat multiple parallel plans as persistent Tracks, maintaining each line's goal, current phase, confirmed progress, blockers, next steps, and associated context; keep mastering progress across terminal instances, multiple turns, and interruptions, so the user does not lose work state between lines.

**Work-state understanding and attention management**: understand work currently happening, terminal changes, waits, completions, risks, and items that need decisions; filter noise that is not worth interrupting over, and when the user's attention is truly needed, promptly explain the reason, impact, and next step.

**Collaborative advancement and review**: help the user understand the current state, organize next steps, advance plans, and after follow-ups or interruptions review "what just happened" and restore the relevant work line and context; also form daily reports, weekly notes, and project phase reviews from the continuously maintained Tracks, consolidating confirmed progress, changes, blockers, and follow-up priorities.

**Pattern automation and quick replies**: understand what the user is currently building through Context Engineering and what it does, e.g. Skill; on that basis gradually provide quick-reply automation, helping the user type fewer repetitive, low-value words, while not making high-risk decisions for the user and not overriding the user's established context, constraints, and working style.

**Multi-tool understanding and visual presentation**: understand the capabilities, boundaries, and interrelationships of the existing tools, and make these visualization tools connect more tightly; at the right moments, through jumps, timely display of relevant content, and organizing context, help the user directly see, enter, and operate the information needed for the current work, rather than only describing it in text.
