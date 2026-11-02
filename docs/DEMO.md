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
Build the website for MemoryOS in this folder with React and TypeScript on Vite. One component per
section, plain CSS with variables for the design tokens, no UI library or CSS framework. It will be
hosted on GitHub Pages, so `npm run build` must produce a static site in `dist/` (set Vite's `base`
to "./"). This is the product's main site, not a short
landing page: a long, polished page like the sites of leading developer-tool companies, with a
real section for every part of the product. It must work on a phone.

Look (decided by the user, keep it everywhere):
- Colors: Mocha Brown #5B4632, Champagne Gold #F4D9A0, Burnt Amber #C8792C, on a black
  background (#0A0806, a warm black). Gold for headings and highlights, amber for primary buttons
  and accents, mocha for rules, lines and quiet surfaces. Body text warm off-white (#F3ECE2) and
  muted text #B5A595.
- Font: Apple's system font. Use the stack -apple-system, BlinkMacSystemFont, "SF Pro Display",
  "SF Pro Text", "Helvetica Neue", Arial, sans-serif, and "SF Mono", ui-monospace, Menlo,
  monospace for code. Do not load a web font.
- Hero: black background, the word "MemoryOS" very large, filled with a gold-to-amber gradient and
  a slow light sweep across it. Behind it, the site's signature: a brain drawn as a network of
  nodes and links (a few hundred gold and amber points inside a brain silhouette, linked to their
  near neighbours), rendered in 3D on a canvas and turning slowly, with the links softly pulsing.
  It is MemoryOS's graph of memories made visible. No photos.
- Git lines: the MemoryOS app draws its timeline as git branch lines, thin vertical rails with
  small hollow commit circles and curves where a branch joins. Use that language across the site:
  one thin rail runs down the left of the page through every section, with a commit circle at
  each section heading; in "How it works" and the handoff story, separate lanes for Claude Code,
  Codex and Muse Code (gold, amber and a light mocha) curve into one shared line. Draw them in SVG
  and let them draw themselves on scroll.
- No boxes: do not put content in cards or bordered boxes. Lay sections out with typography,
  whitespace, thin rules and the git lines. The only framed things are the app screenshots.
- No status dots, pulsing dots or green dots anywhere.
- Calm and premium: generous spacing, subtle gradients only where they help.
- Motion: every section animates in on scroll (fade and rise, children staggered, using
  IntersectionObserver); result numbers count up when they come into view; the nav turns
  translucent with a blur once you scroll; links and screenshots respond gently on hover; the
  node brain drifts slowly with the scroll. All of it smooth and subtle, and off under prefers-reduced-motion.
- Real product images: public/screenshots/ holds screenshots of the MemoryOS app (graph.png,
  timeline.png, review.png, requests.png, documents.png, agents.png; 2880x1800). Use them, never
  placeholders: a large framed graph.png under the hero buttons, and an app tour section with tabs
  that switch between the screenshots. Show them in a macOS-style window frame with a soft gold
  glow.
- Type like Apple's product pages: very large display headings (up to about 120px on desktop,
  tight letter-spacing), short lines, lots of air between sections. Keep the weight light: display
  headings at 500 or lighter (never bold or heavy), body text at 400.

Quality bar: this must look like a funded company's site, not a template. Write readable,
formatted code. Before you finish, run the site and look at every section at 1440px and 390px
wide; fix anything that overlaps, feels empty or looks unfinished. Keep the repository to the
site's code and a short README.

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
5. Features, as an open list with the git lines, not cards: shared across agents; memory that updates itself (repeats
   strengthen, replacements retire the old memory with undo); knows who said it (a user's
   decision beats an agent's guess); flags memories that conflict with the code; a map of the
   code, its endpoints and services; date-aware search ("last May", "10 days ago"); review and
   undo every link; private by default.
6. Beyond retrieval: a two-column comparison, plain retrieval vs MemoryOS.
7. Results: with MemoryOS 39/42 agent tasks passed vs 26/42 without; LongMemEval: evidence in the
   top 20 for 100% of questions, 86.7% answered correctly; search 23 ms median, 45 ms over one
   million chunks. Big numbers, with one line under each saying how it was measured.
8. The desktop app: a tour of the real screenshots (graph, timeline, review, requests, agents).
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

Run `npm run dev` in the folder, open the address it prints, and scroll through the site. Then show the graph: Codex's and Muse's memories
on one project.

## If something goes wrong

- Codex does not mention MemoryOS: the hook was not trusted. Run `/hooks`, trust it, start a new
  Codex session.
- Muse asks what the website is: check the Requests tab for its `search`. If there is none, the
  project folder was not added to MemoryOS before take 1.

## Starting over

1. In MemoryOS, remove **memoryos-site** (the x next to it) and choose **Delete its memory**.
2. Reset the folder; the committed app screenshots stay:
   ```bash
   cd ~/Projects/Hackathons/Meta/memoryos-site && git clean -fdx
   ```
3. Add the folder in MemoryOS again. It starts with only the screenshots commit.
