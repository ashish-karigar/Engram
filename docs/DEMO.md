# Demo: Codex starts the website, Muse Code finishes it

One project, two agents from two companies, no hand-written notes between them. Codex builds the
first half of the MemoryOS website and stops. Muse Code is told only "finish the website"; what was
decided and what is left reaches it through MemoryOS.

## Setup (once, before recording)

1. Rebuild and open the MemoryOS app (commit 2004521 or later).
2. Agents tab: connect **Codex** and **Muse Code**. Codex now also gets a SessionStart hook in
   `~/.codex/hooks.json`.
3. Codex asks you to review the new hook once: run `/hooks` in Codex and trust the MemoryOS hook.
   Codex skips untrusted hooks.
4. Add the folder `~/Projects/Hackathons/Meta/memoryos-site` as a project in MemoryOS. It is an
   empty git repository.

## Take 1: Codex (in the ChatGPT app, or `codex` in a terminal)

Open the `memoryos-site` folder in Codex and paste:

```text
Build the marketing website for MemoryOS in this folder: plain HTML and CSS, no build step, no
frameworks, so it can be served by GitHub Pages. It must look professional: calm, minimal, dark
neutral grey (like Linear or Vercel), one accent blue (#8DB8FF), IBM Plex Sans for text and
JetBrains Mono for small labels, generous spacing, works on a phone.

MemoryOS is local-first shared memory for coding agents. It watches a project's commits, agent
sessions and documents, keeps the decisions, reasons and dead ends, and serves them to Claude Code,
Codex and Muse Code over MCP, so every agent starts where the last one stopped. Everything stays on
the user's machine. Made by Ashish Karigar (Instagram @ashish.karigar).

Sections, in order:
1. Nav + hero: name, one-line pitch, "Download for macOS" and "Watch the demo" buttons.
2. Problem: agents forget between sessions; each agent is an island; dead ends repeat.
3. How it works: Capture, Understand, Map, Recall.
4. Results: with MemoryOS 39/42 agent tasks passed vs 26/42 without; LongMemEval 100% of evidence
   in the top 20, 86.7% of answers correct; search 23 ms median.
5. Demo video (embed placeholder).
6. Footer: "~Ashish Karigar" linking to Instagram, GitHub link.

We only have time for part of it today. Do sections 1 to 3 well, and leave 4 to 6 for the next
session. Make the design decisions now so the rest stays consistent.
```

Let it finish. In the app, show the Timeline: Codex's decisions (palette, fonts, layout, what is
left) arrive as memories.

## Take 2: Muse Code (terminal)

```bash
cd ~/Projects/Hackathons/Meta/memoryos-site && muse
```

Paste only:

```text
Finish the website.
```

Muse Code gets the brief at session start, searches MemoryOS for the remaining sections, and builds
4 to 6 in the same style. Show the Requests tab: Muse's searches and what they returned.

## Take 3: the result

Open `index.html` in a browser and scroll through it. Then show the graph: Codex's and Muse's memories
on one project.

## If something goes wrong

- Codex does not mention MemoryOS: the hook was not trusted. Run `/hooks`, trust it, start a new
  Codex session.
- Muse asks what the website is: check the Requests tab for its `search`. If there is none, the
  project folder was not added to MemoryOS before take 1.
