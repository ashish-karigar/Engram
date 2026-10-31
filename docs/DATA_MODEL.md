# Engram's code knowledge model

The map, impact and every answer about code are only as good as what is stored underneath. This is
the model: four layers over the same entities, each edge typed, weighted, time-bounded and traceable
to where it came from.

## Entities

One table, `entities(kind, qualified_name, path, …)`, for everything code-shaped. Kinds form a
containment hierarchy, so any view can zoom from the whole system to one function:

```
repository
└─ area        a part with its own manifest: Angular app, FastAPI service, Rust crate, iOS app
   └─ module   a folder inside the part
      └─ file
         └─ symbol   class, function, method, struct, trait, … (methods nest in their class)
endpoint       an HTTP route, e.g. PATCH /jobs/{}
service        something outside the repository: Auth0, Ollama, Gmail API, Meta API
```

Symbol details live beside the entity in `code_symbols`: file, name, kind, line span, parent
(the class a method belongs to), language and signature (the declaration line). Spans let anything
that knows a line number (a call site, a diff hunk, a blame line) find its function.

## Layer 1: structure

`contains` (area → module → file → symbol, class → method) and `in_area` (file → area). Rebuilt on
every index pass from the parsers; never edited by hand.

## Layer 2: dependencies, what breaks what

| Edge | From → to | Source | Confidence (edge weight) |
|---|---|---|---|
| `imports` | file → file | parsed imports, resolved to project files | 0.8 |
| `invokes` | function → function | parsed call sites, resolved by name through the caller's file, class and imports | 1.0 unique match, 0.5 when 2–3 candidates; more is skipped |
| `serves` | function → endpoint | route decorators and routers | 0.9 |
| `calls` | function → endpoint or service | HTTP client calls, resolved paths | 0.9 |
| `uses` | file → service | SDK imports, client base URLs | 0.7 |

Raw facts are kept per file (`net_routes`, `net_calls`, `net_services`, `code_calls`) and edges are
derived from them, so a re-index of one file refreshes its facts and the links are rebuilt, with no
drift. Impact walks these edges backwards.

## Layer 3: provenance, who wrote what

`commits(sha, ts, author, subject, files, agent, session, evidence)` and
`code_history(entity_id, commit_sha, lines)`:
- **From git:** each commit's author, time, message and files; per file and symbol, which commits
  wrote its current lines (`git blame` mapped onto line spans, re-run only for files that changed).
  Lines not committed yet are counted as "uncommitted".
- **From agent sessions:** every Write/Edit tool call is an event with the agent, session and files
  (Codex edits are read from `apply_patch` in its shell commands). A commit is attributed to an agent
  with the evidence shown next to it: **edited** (that agent edited the commit's files since the
  previous commit), **trailer** (a co-author line names it), or **session** (one of its sessions was
  active here in the 30 minutes before: likely, not certain). Otherwise it stays with its git author.
- **Summary on each file and symbol:** lines per author or agent, created in (commit, date, who), last
  changed in, number of commits, and the agent sessions involved, each linked to its transcript.
  Given to agents in `expand` and `impact`, shown in the map's drawer and as "colour by author".

## Layer 4: knowledge

`memories` (decisions, facts, dead ends, …) `touches` the files and symbols they are about;
`documents` `describes` their own file and `mentions` others; agent conversations are documents.
All bitemporal: a superseded decision keeps its history.

## Views over the model

- **Network:** one Obsidian-style 3D graph of everything, laid out as a solar system per part: the
  part is the sun, its files the planets, their functions and classes the moons (methods orbit their
  class). Each API is a rocket: one ball with its endpoints, pulled by the code that calls it and the
  code that serves it, so it hangs between the two suns. External services are spaceships near the
  code that uses them; memories and documents sit by what they are about. Containment holds firm and
  links across parts pull softly, so each part keeps its own system; hiding a kind (say functions)
  hands its links to the parent, so the network never falls apart. The camera starts with the two
  biggest suns side by side. Lines are one quiet colour at rest; selecting a node colours its lines
  by type, with dots flowing from caller to callee. Colour by type or by author.
- **Impact:** a selection lists everything that depends on it, at function level.
