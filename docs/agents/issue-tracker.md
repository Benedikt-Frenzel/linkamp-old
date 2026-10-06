# Issue tracker: GitHub Issues

Issues and specs for this repo live as GitHub Issues in `Benedikt-Frenzel/linkamp`, managed with `gh`.

## Conventions

- One label per feature area is NOT used; the feature is named in the issue title: `Spec: <feature>`, `<Feature> NN: <ticket title>`.
- The spec issue body carries Problem Statement, Solution, User Stories, Implementation Decisions, Testing Decisions, Out of Scope, and Further Notes.
- Implementation tickets reference their feature by title prefix and number, e.g. `Playback 03: …`. A `Blocked by` section lists the gating tickets by number and title.
- Acceptance criteria are GitHub task lists; comments and conversation history append as issue comments.
- Triage state is a label (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`), matching `triage-labels.md`.

## When a skill says "publish to the issue tracker"

Create a GitHub issue with `gh issue create -R Benedikt-Frenzel/linkamp`.

## When a skill says "fetch the relevant ticket"

Read the issue with `gh issue view <number> -R Benedikt-Frenzel/linkamp --comments`. The user will normally pass the number or URL directly.

## Wayfinding operations

Used by `/wayfinder`. The **map** is an open issue; **child tickets** are issues referencing it.

- **Map**: an open issue titled `<effort> map`, body sections Notes / Decisions-so-far / Fog.
- **Child ticket**: an issue referencing the map, with a `Type:` line (`research`/`prototype`/`grilling`/`task`) and a `Status:` line (`claimed`/`resolved`) in the body. A `Blocked by: NN, NN` line lists gating tickets.
- **Frontier**: `gh issue list -R Benedikt-Frenzel/linkamp --state open`, first by number wins among unblocked, unclaimed.
- **Claim**: edit the body to `Status: claimed` and save before any work.
- **Resolve**: comment the answer under the issue, set `Status: resolved`, and append a context pointer to the map issue's Decisions-so-far.
