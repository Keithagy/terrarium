# Terrarium — design notes

## Subject and job

A terrarium: a glass jar you keep on a desk and look into now and then to see how the
small world inside is doing. The product is a desktop app that developers (and their
agents) open on a repository they do not know yet. Its job is to let them reason about
what the software does and how it does it without reading the code. The world inside the
jar is an atlas: C4 diagrams of the system at four levels, drawn from the code and
annotated by agents, with every arrow backed by evidence.

C4 gives the vocabulary. The **context** level shows who uses the system and what it
talks to. **Containers** are the things that run: web apps, services, workers, command
lines, libraries. **Components** are the parts inside one container. **Code** is one
component's files and the parts in them. A **journey** is one request followed across the
diagrams, numbered step by step.

## How an atlas is made

The scanner turns the repository into a graph (packages, files, symbols; imports, calls,
cross-language flows). The engine draws a plain atlas from the graph alone: packages
become containers, directories become components, imports and calls become
relationships, env vars and boundary tags reveal databases, queues and file systems. It
is correct and dull: names come from folders.

Discovery hands the words to agents, in three stages, and checks every one:

1. **Survey.** One agent reads the manifests, README and entry points and decides what the
   system is, who uses it, what it depends on, and what each package runs as.
2. **The field.** One agent per container reads that container's code and groups it into
   components, each named by what it does, with the relationships it sees. One agent per
   journey follows a trace the scanner found and writes what happens at every crossing.
   They run in parallel, and every file an agent opens is reported as it happens.
3. **The editor.** One agent, no tools, writes the summary, where to start and what is
   worth knowing.

Then the engine verifies. A component only holds files that exist in its container;
files an agent forgot go into "Other files". A relationship is **backed by code** when
the graph has an import, call or flow behind it (the evidence is kept and shown), **from
the survey** when it is the kind of thing code cannot show (a person, a third-party
system), or **claimed** when an agent asserted it and the code does not: those are drawn
dashed and marked, never dropped and never passed off as fact. The atlas is saved as JSON
and can be exported as Structurizr DSL.

## Tokens

Colour, a green-glass base with several warm accents rather than one:

| token | value | role |
| --- | --- | --- |
| glass | `#17211C` | window base, seen through macOS vibrancy |
| glass-2 / glass-3 | `#1F2A24` / `#26332C` | panels, controls, box fills |
| mist | `#D9E4DA` | primary text, arrows backed by code |
| fern / fern-dim | `#8FA396` / `#5F7266` | secondary text, survey arrows, outside-system outlines |
| lamp | `#F2B950` | the one warm accent: journeys, selection, the system's outline, agents at work |
| rust | `#E0904A` | claimed arrows and anything the code does not back; also the Rust hue |
| rust / typescript / javascript / python / go / other | `#E0904A` `#6FB3E0` `#E6D25A` `#8FBF6A` `#5ED3C0` `#A99AC9` | a container's tint, by the language most of its lines are in |

Type: **Fraunces** (variable, optical size and SOFT axes) for names: the system, the
containers, the components, the crumbs, the card title; **IBM Plex Sans** for everything
read; **IBM Plex Mono** only where the content is code (paths, tags, the file an agent is
reading). Fonts are bundled in `app/ui/public/fonts`.

Layout: the diagram sits on a stud-dotted plate in the middle of the window, with
floating glass panels around it, each shaped for its job. The header (top: the system's
name and counts: containers, components, relationships, how many are backed by code,
how many claimed, who wrote the atlas). The shelf (left: search, the map of containers
and components, the journeys, the guide). The crumbs (top of the plate: where you are,
and the four levels). The card (right, an amber top edge: the selected element or
relationship, with its evidence). The field notes (right, in place of the card, while
agents work). The journey bar (bottom: step through one journey).

```
┌────────────────────────────────────────────────────────────────────────────┐
│ Polyglot Town  4 containers 10 components 26 relationships 24 backed  [Discover with Claude] │
│ ┌ shelf ────┐ Polyglot Town › Containers      Context Containers Components Code  ┌ card ──────┐ │
│ │ search    │              (Developer)                                     │ Users API   │ │
│ │ Map       │                  │ uses                                      │ Service · Py│ │
│ │  ▸ Web    │           ┌──────┴─────┐  http /api/users  ┌──────────┐      │ what it does│ │
│ │  ▸ API    │           │ Web front  │──────────────────▶│ Users API│──SQL▶│ talks to    │ │
│ │ Journeys  │           └────────────┘                   └──────────┘      │ evidence    │ │
│ │ Guide     │                                              (PostgreSQL)    │ [Open]      │ │
│ └───────────┘ ┌ journey ─────────────────────────────────────────────────┐ └─────────────┘ │
│               │ ‹ › Sign up   Step 3 of 11   Web front end → Users API … │                 │
└────────────────────────────────────────────────────────────────────────────┘
```

The diagram's grammar: a person is a rounded box with a head; the system, at the context
level, has the lamp outline; a container is a box tinted by its language with a coloured
edge on the left, its kind and technology under the name and two lines of description; a
component is the same, smaller; an outside system is a dashed outline, a database a
cylinder, a queue a pipe. Neighbours on a component diagram (other containers this one
talks to) are dashed and quiet. Arrows curve from the bottom of one box to the top of the
next and carry a label pill saying what passes; a claimed arrow is dashed in rust with the
word on it. Selecting an element dims everything not connected to it. A journey lights
its arrows amber, numbers them, and dims the rest.

## Discovery, live

The one orchestrated motion in the app is discovery. When it starts, the field notes
panel opens and the header button becomes a live pill (agents done, dollars spent,
seconds). The survey lands first: the system's name replaces the folder name in the
header, containers rename themselves and their kinds settle, and each renaming box flashes
amber once. Then the field: a container whose agent is reading gets a marching amber
outline, the name of the file being read under its description, and a tick per file read;
each read pings the box. As each agent finishes, its container develops: components,
description, responsibilities. Journeys appear in the shelf as their narrators finish.
The notes log narrates all of it in plain sentences with timestamps, and ends with the
check: how many relationships are backed, how many the survey declared, how many are
claimed. Nothing else in the app moves unless asked.

## Principles

- The diagram is the hero. Panels are translucent and quiet; nothing on them glows.
- Boldness is spent once: amber marks journeys, selection, the system's outline and
  agents at work. Rust marks what the code does not back. Boxes carry the six language
  hues; the plate and the arrows stay muted.
- Structure encodes information: a box's tint is its language, its shape is its kind,
  an arrow's stroke is how much to trust it, and a journey's numbers are its order.
  Nothing is placed at random: rows are dependency layers, people on top, outside
  systems at the bottom.
- Every arrow is accountable. Click one and the card shows the imports, calls, flows or
  tags behind it, each opening the file in an editor. An agent can add words to an
  arrow; it cannot add an arrow and have it pass for fact.
- Motion only answers an action or reports work in progress: a level change eases in,
  a journey step lights up, an agent's reads ping. `prefers-reduced-motion` disables all of it.
- Words are plain verbs: "Open a repository", "Discover with Claude", "Read the code",
  "Show the whole path". Empty and error states say what to do next.

## Review against the defaults

The near-black-plus-one-neon-accent look was the obvious default and was rejected: the
base is a tinted green with real depth from vibrancy, and colour carries meaning
(language, trust) across several hues. Standard C4 tooling draws every box the same blue;
here shape and tint do work. No all-caps eyebrow labels. Numbered markers appear only on
journeys, where the number is the content. Middle dots appear only in counter lines,
where they separate a list. Rounded corners differ by role (16px panels, 14px containers,
9px controls, pills for status and labels) rather than one radius everywhere. The live
discovery view was designed as one moment rather than scattered spinners: one panel,
one pill, and the boxes themselves showing what is being read.
