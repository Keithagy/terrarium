# Terrarium — design notes

## Subject and job

A terrarium: a glass jar you keep on a desk and look into now and then to see how the
small world inside is doing. The product is a desktop app that developers (and their
agents) open to see the shape of a polyglot repository: which packages exist, how files
rest on each other, and where data crosses language boundaries. The world inside the jar
is a brick model of the repository, built step by step, because a model you can watch go
together teaches the order to read the code in. A node graph shows that things are
connected, but a node's position means nothing; in the model, position means district,
height means size, and the build order means dependency.

## Tokens

Colour, a green-glass base with several warm accents rather than one:

| token | value | role |
| --- | --- | --- |
| glass | `#17211C` | window base, seen through macOS vibrancy |
| glass-2 / glass-3 | `#1F2A24` / `#26332C` | panels, controls |
| mist | `#D9E4DA` | primary text |
| fern / fern-dim | `#8FA396` / `#5F7266` | secondary text, import edges |
| lamp | `#F2B950` | the one warm accent: bridges (data flows), lamps, selection, primary button, step numbers |
| rust / typescript / javascript / python / go / other | `#E0904A` `#6FB3E0` `#E6D25A` `#8FBF6A` `#5ED3C0` `#A99AC9` | brick colour by language |

Type: **Fraunces** (variable, optical size and SOFT axes) for the model's title, step
titles, sub-build names and card titles; **IBM Plex Sans** for UI text; **IBM Plex Mono** only
where the content is code (paths, ids, tags). Fonts are bundled in `app/ui/public/fonts`.

Layout: the model sits in the middle of the window on canal water, with floating glass
panels around it, each shaped for its job. The header (top: the model's name and chips
like a brick set's box: pieces, steps, sub-builds, studs, joints, "holds together"). The
shelf (left, lists: sub-builds, traces, endpoints). The stage tabs (Model, Manual, Parts,
Traces, Design). The timeline (bottom: play the build at 0.5×–4×, scrub, first and last).
The specimen card (right, an amber top edge, only when something is selected), or the
manual page in its place on the Manual tab.

```
┌─────────────────────────────────────────────────────────────────────┐
│ Polyglot Test Town  27 pieces 12 steps … holds together  [Design with Claude] │
│ ┌ shelf ─────┐ [Model Manual Parts Traces Design]   ┌ page / card ─┐ │
│ │ search     │ [Iso Front Top Spin Fit]             │ 6  Rust crate │ │
│ │ sub-builds │        brick model on canal water    │ Expose        │ │
│ │ traces     │     districts · towers · bridges     │ scan_repo …   │ │
│ │ endpoints  │                                      │ parts, rests  │ │
│ │ legend     │ ┌ timeline ──────────────────────────┴───────────────┐ │
│ └────────────┘ │ ⏮ ▶ ⏭  0.5× 1× 2× 4×  Step 6 of 12 ──●────────── │ │
└─────────────────────────────────────────────────────────────────────┘
```

The model's grammar: a package is a district (a stud baseplate tinted toward sand and
stone), a file is a building standing on it, and each function, type or constant is one
brick in that building, stacked in source order (files with more than 14 share bricks).
Cross-language flows are amber arched bridges from the calling brick to the handling
brick. A file that starts a trace carries an amber lamp on its roof. Districts pack onto
one baseplate with two-stud canals between them.

The manual adds files in dependency order, so each step only rests on steps before it.
The Manual tab's page names the chapter (sub-build), the step's title and caption, the
parts it adds (the bricks, by name), what they rest on (links to earlier steps), and the
bridges the step completes. The Parts tab is the parts list from the front of a set. The
Design tab says who designed the manual (the engine, or Claude agents) and shows the
joint check.

The Traces tab keeps the lane diagram for one request: one lane per package, one row per
step in call order, calls as quiet fern connectors and each boundary crossing as an
amber line into the next lane, labelled with its route or command. "Show on model"
dims every building the trace does not pass through.

## Principles

- The model is the hero. Panels are translucent and quiet; nothing on them glows.
- Boldness is spent once: amber marks data crossing languages (bridges), landmarks
  (lamps) and where you are (selection, the current step). Bricks carry the six
  language hues; plates and water stay muted.
- Structure encodes information, and nothing is placed at random: district is package,
  height is how much a file defines, build order is what rests on what. External
  dependencies are left out, because they are not part of what you are reading.
- Every piece must hold. The engine checks each design joint by joint and repairs what an
  agent placed too early, and the header says whether the model holds together.
- Motion only answers an action: pieces dropping in as the build plays, the camera easing
  to a district, the turntable when asked. `prefers-reduced-motion` disables the easing.
- Words are plain verbs: "Open a repository", "Start building", "Trace from here".
  Empty and error states say what to do next.

## Review against the defaults

The near-black-plus-one-neon-accent look was the obvious default here and was rejected:
the base is a tinted green with real depth from vibrancy, and colour carries meaning
(language) across six hues. No all-caps eyebrow labels. Step numbers are the one place
numbered markers appear, because in a build manual the number is the content. Middle
dots appear only in counter lines, where they separate a list. Rounded corners differ by role (16px panels, 9px controls,
pills for status) rather than one radius everywhere.
