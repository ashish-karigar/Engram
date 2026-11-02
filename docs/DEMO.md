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
Build the website for MemoryOS in this folder: plain HTML, CSS and a little JavaScript, no build
step and no frameworks, so GitHub Pages can serve it. This is the product's main site, not a short
landing page: a long, polished page like the sites of leading developer-tool companies, with a
real section for every part of the product. It must work on a phone.

Look (decided by the user, keep it everywhere):
- Colors: Mocha Brown #5B4632, Champagne Gold #F4D9A0, Burnt Amber #C8792C, on a black
  background (#0A0806, a warm black). Gold for headings and highlights, amber for primary buttons
  and accents, mocha for cards, borders and quiet surfaces. Body text warm off-white (#F3ECE2) and
  muted text #B5A595.
- Font: Apple's system font. Use the stack -apple-system, BlinkMacSystemFont, "SF Pro Display",
  "SF Pro Text", "Helvetica Neue", Arial, sans-serif, and "SF Mono", ui-monospace, Menlo,
  monospace for code. Do not load a web font.
- Hero: black background, the word "MemoryOS" very large, filled with a gold-to-amber gradient and
  a slow light sweep across it, a soft amber glow behind it. No photos. Respect
  prefers-reduced-motion.
- Calm and premium: generous spacing, thin mocha borders, subtle gradients only where they help.

MemoryOS is local-first shared memory for coding agents. It watches a project's commits, agent
sessions and documents, keeps the decisions, reasons and dead ends, and serves them to Claude Code,
Codex and Muse Code over MCP, so every agent starts where the last one stopped. Everything stays on
the user's machine. Made by Ashish Karigar (Instagram @ashish.karigar).

Sections, in order:
1. Sticky nav: MemoryOS, links to the sections, a "Download for macOS" button.
2. Hero: "MemoryOS", the line "Shared memory for every coding agent you use", buttons
   "Download for macOS" and "Watch the demo", and a row "Works with Claude Code · Codex · Muse Code".
3. The problem: every session starts cold; every agent is an island; dead ends repeat.
4. How it works: Capture, Understand, Map, Recall, as four connected steps.
5. Features, as a grid of cards: shared across agents; memory that updates itself (repeats
   strengthen, replacements retire the old memory with undo); knows who said it (a user's
   decision beats an agent's guess); flags memories that conflict with the code; a map of the
   code, its endpoints and services; date-aware search ("last May", "10 days ago"); review and
   undo every link; private by default.
6. Beyond retrieval: a two-column comparison, plain retrieval vs MemoryOS.
7. Results: with MemoryOS 39/42 agent tasks passed vs 26/42 without; LongMemEval: evidence in the
   top 20 for 100% of questions, 86.7% answered correctly; search 23 ms median, 45 ms over one
   million chunks. Big numbers, with one line under each saying how it was measured.
8. The desktop app: graph, review, timeline and agents, with image placeholders.
9. Privacy and security: listens only on this Mac, a separate key for each agent, keys in files
   only you can read, reset a key in one click, nothing uploaded.
10. Get started: three steps (download, connect your agents, add a project) with copyable commands.
11. FAQ: six questions.
12. Demo video (embed placeholder) and a closing call to action.
13. Footer: "~Ashish Karigar" linking to Instagram, GitHub link, license "All rights reserved".

We only have time for part of it today. Build the whole structure and design system, and do
sections 1 to 6 fully. Leave 7 to 13 for the next session. Make the design decisions now so the
rest stays consistent.
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
7 to 13 in the same style. Show the Requests tab: Muse's searches and what they returned.

## Take 3: the result

Open `index.html` in a browser and scroll through it. Then show the graph: Codex's and Muse's memories
on one project.

## If something goes wrong

- Codex does not mention MemoryOS: the hook was not trusted. Run `/hooks`, trust it, start a new
  Codex session.
- Muse asks what the website is: check the Requests tab for its `search`. If there is none, the
  project folder was not added to MemoryOS before take 1.
