# Terrarium — design notes

## Subject and job

A terrarium: a glass jar you keep on a desk and look into now and then to see how the
small world inside is doing. The product is a desktop app that developers (and their
agents) open to see the shape of a polyglot repository: which packages exist, how files
depend on each other, and where data crosses language boundaries. Its job is to make that
shape legible in one glance and let you dig into one specimen at a time.

## Tokens

Colour, a green-glass base with several warm accents rather than one:

| token | value | role |
| --- | --- | --- |
| glass | `#17211C` | window base, seen through macOS vibrancy |
| glass-2 / glass-3 | `#1F2A24` / `#26332C` | panels, controls |
| mist | `#D9E4DA` | primary text |
| fern / fern-dim | `#8FA396` / `#5F7266` | secondary text, import edges |
| lamp | `#F2B950` | the one warm accent: data flows, selection, primary button |
| rust / typescript / javascript / python / go / other | `#E0904A` `#6FB3E0` `#E6D25A` `#8FBF6A` `#5ED3C0` `#A99AC9` | node colour by language |

Type: **Fraunces** (variable, optical size and SOFT axes) for the repo name, node titles
and package labels on the canvas; **IBM Plex Sans** for UI text; **IBM Plex Mono** only
where the content is code (paths, ids, tags). Fonts are bundled in `app/ui/public/fonts`.

Layout: the canvas is the whole window. Three floating glass panels, each a different
shape for a different job: the shelf (left, a list), the specimen card (right, a sheet
with an amber top edge that appears only when something is selected), the bench (bottom
strip, status and mode). Everything left-aligned.

```
┌────────────────────────────────────────────────────────────────┐
│ ┌ shelf ─────┐                              ┌ specimen ──────┐ │
│ │ search     │          canvas              │ api.ts         │ │
│ │ packages   │     moss patches, discs,     │ tags, facts    │ │
│ │ flows      │     amber flows              │ used by / uses │ │
│ │ legend     │                              └────────────────┘ │
│ └────────────┘                                                 │
│ bench: repo  [packages files symbols]  layout ●   counts  fps  │
└────────────────────────────────────────────────────────────────┘
```

Two stages fill the space behind the panels. **Traces** (the default when a repository
has any) follows one request from its entry point to where its data comes to rest: one
lane per package, one row per step in call order, calls as quiet fern connectors and
each boundary crossing as an amber line into the next lane, labelled with its route or
command. Position means something here — left to right is who hands data to whom, top to
bottom is call order — which the force-directed **map** cannot say. The map stays for
the shape of the whole repository, and "Show on map" lights a trace up on it.

```
│ ┌ shelf ─────┐  main  web/src/app.ts                            │
│ │ traces     │  Crosses 4 boundaries through TypeScript, …      │
│ │ endpoints  │  polyglot-web     polyglot-api     worker        │
│ │ packages   │  [main]                                          │
│ │ boundaries │   └[fetchUsers]──http /api/users──▶[get_users]   │
│ └────────────┘                                     └[list_users db]
```

The Endpoints tab is the contract check: routes nothing in the repository calls, and
calls nothing in the repository serves, sorted first.

## Principles

- The graph is the hero. Panels are translucent and quiet; nothing on them glows.
- Boldness is spent once: data flows are amber and thicker, with a slow travelling light
  when they belong to the selected node. Everything else is desaturated green.
- Structure encodes information: package membership is a soft moss patch under the
  nodes, not a box; edge direction is a fade toward the target, not an arrowhead;
  external dependencies are small and dim because they are leaves.
- Motion only answers an action: the layout settling after a scan, the camera easing to
  a selection. No entrance animations. `prefers-reduced-motion` disables the easing.
- Words are plain verbs: "Open a repository", "Show 4 inside", "Settling the layout".
  Empty and error states say what to do next.

## Review against the defaults

The near-black-plus-one-neon-accent look was the obvious default here and was rejected:
the base is a tinted green with real depth from vibrancy, and colour carries meaning
(language) across six hues. No all-caps eyebrow labels, no numbered markers, no
middle-dot metadata strings in the panels (the bench uses them once, as a counter line,
where they are a list). Rounded corners differ by role (16px panels, 9px controls,
pills for status) rather than one radius everywhere.
