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
- **From git:** per commit, which files changed (`git log --numstat`); per symbol, which commits
  touched its lines (`git blame` over its span, only for files that changed since the last pass).
- **From agent sessions:** every Write/Edit tool call is already an event with the agent, session
  and file. A commit whose files an agent edited in the window before it is attributed to that agent
  and session; otherwise to the person who committed it. Uncommitted agent edits are attributed the
  same way against the snapshot ref.
- **Summary on each symbol:** created in (commit, author, agent), last changed in, number of changes,
  number of authors and agents. Shown in the map and given to agents with `impact`, so "who wrote
  this, in which session, and why" leads to the conversation and decision behind it.

## Layer 4: knowledge

`memories` (decisions, facts, dead ends, …) `touches` the files and symbols they are about;
`documents` `describes` their own file and `mentions` others; agent conversations are documents.
All bitemporal: a superseded decision keeps its history.

## Views over the model

- **Architecture:** parts, endpoints, services, memories (built).
- **Zoom:** double-click a part to open its folders, a folder its files, a file its classes and
  functions; edges are summed at each level (e.g. "150 calls from `app/` to `services/`") and drawn
  thicker for more. Colour by type or by author (who wrote most of each node).
- **Impact:** a selection lights up everything that depends on it, at function level.
