# DESIGN.md — `rusdd` architecture decisions

This document records the rationale behind specific key design choices.
Choices made during the development of `rusdd` - a "really useful" imaging tool.
It may serve as a reference for a curious developed wandering GitHub.
It is a learning artifact of my journey to Rust-land (hence the name).
It can be also viewed as an example of human-AI collaborative design process.

I wrote all the code myself (with autocompletion of course) because:
+ ~20 years of exposure to programming taught me to OWN my decisions
+ I really want to learn Rust deeply so mechanical memory is a must
+ I wanted to rekindle my passion for programming and it did happen!
+ It is a real pleasure to code in Rust inside Zed editor build with Rust:)

I worked with DeepSeek AI as my co-designer because:
+ I needed 'a rubber duck' to talk to (without feeling outdated)
+ It is unsurprisingly good at converting well-worded ideas to code snippets
+ I don't have neither money nor time for the most fancy new agents/models/shit

> In essence, I reject 'vibe-coding'.
> I was lucky to get proper (ACM-ICPC-like) education ~20 years ago.
> But turns out LLMs are pretty good productivity multiplier.
> If used to compliment natural intelligence.
> Not replace it with prompt gambling.

For example it formatted my explanations about key pivots in logic.
I then read it and refined so the text below is not simpy copy-pasted.
Here they are - overall design philosophy and architectural decision records (ADR).
For all of us (human or AI) to read and get more context about the tool.


## Design philosophy

`rusdd` is built on a few core principles that should guide ADR:

1. **Single-pass when possible.**
> The tool reads source data once for the primary copy;
> only damaged regions get a second read.
2. **Stdlib as a constraint, not a limitation.**
> Every missing feature (device size query, fancy CLI) is an opportunity 
> to learn what the OS actually provides and how to work around gaps.
3. **The layout is the universal interface.**
> Whether inspecting, copying, or comparing, the output is always a CSV
> segment map. Humans read the comments; machines parse the hex columns.
4. **Optimize for the common case.**
> Many SD cards could be partially empty in the real-world workflows.
> Ugly segment deferral and truncation make the tool faster in such scenarios.

---

### ADR-001: Pure stdlib only [2026-04-02^06]

**Context:**
* Primary goal is learning Rust. 
* External crates reduce learning surface. 
* The tool must work on Windows and Unix without platform-specific dependencies.

**Decision:**
* No external crates.
* All functionality built on `std::fs`, `std::io`, `std::env`.

**Consequences:**
- CLI parsing is manual (verbose but educational)
- Raw device size query unavailable - must use heuristic EOF
- Progress reporting is hand-rolled
+ Acceptable for a pet project with real-world utility

---

### ADR-002: Two-pass design (coarse scan + fine recovery) [2026-04-07^25]

**Context:**
* Reading large SD cards sector-by-sector is slow (many syscalls).
* But reading in large chunks means a single bad sector taints the whole chunk.

**Decision:**
* **Pass 0/1 (SCAN+COPY):**
  * Read in large chunks (`--chunk-size`, default 32 MiB).
  * Classify each chunk as Good/Bad/Ugly.
  * Write Good immediately, defer Ugly, placeholder Bad.
  * Build a Layout segment map.
* **Pass 2 (RECOVER):** ==Not yet implemented==
  * Revisit Bad segments at `--sector-size` granularity.
  * Salvage readable sectors.

**Consequences:**
- Two reads of Bad regions (acceptable: Bad regions are typically small)
+ Good regions read once
* Layout must fit in memory (typically small segment count)
- Source must be seekable (true for drives and files)

---

### ADR-003: Heuristic EOF detection [2026-04-26]

**Context:**
* Windows does not expose physical media size via stdlib.
* `SeekFrom::End` is unreliable on raw devices.

**Decision:**
* Detect end-of-media by observing consecutive read errors.
* Threshold: 8 consecutive errors (codes 23 and 27 observed in testing).
* Same error code counts as saturation; changing errors reset the counter.

**Consequences:**
+ Works with tested USB SD-card readers
- False positives possible on heavily damaged media
- False negatives possible on media that returns zeros past end
- Acceptable tradeoff for a no-dependency tool

---

### ADR-004: Ugly segment optimization [2026-04-27^29]

**Context:**
* Empty SD-card space is typically all-zeros or all-0xFF.
* Writing gigabytes of identical bytes is wasteful.

**Decision:**
* Detect Ugly segments as 4-byte repeating patterns (not 1-byte)
* During Copy: defer writing via `PatternRun` accumulator
* On pattern change or traversal end: flush accumulated run in one efficient bulk write
* If `--truncate` is set: 
  * trailing Ugly run is reduced to one stride (the "hint chunk")
  * then the image stops

**Consequences:**
+ Massively reduces write amplification for mostly-empty media
* 4-byte detection catches: zeros, erased flash (0xFF), memory test patterns
+ The hint chunk makes truncated images identifiable as intentionally truncated, not corrupted
+ `PatternRun::flush` writes in 4 KiB blocks - constant memory

---

### ADR-005: Segment merge semantics [2026-04-29]

**Context:** Adjacent strides of the same status should merge into a
single Segment to keep the layout compact. But with rolling checksums,
every Good stride has a different essence. Merging on both status and
essence would produce thousands of tiny segments.

**Decision:** `Layout::push` merges on status, not essence:
- Good merges with Good unconditionally (essence updated to latest)
- Bad merges with Bad unconditionally
- Ugly merges with Ugly only if the pattern matches

**Consequences:**
- Adjacent Good strides form a single segment; stored essence is the
  cumulative checksum at segment end
- Adjacent Bad strides form a single segment
- Ugly runs with different patterns remain separate (0x00 vs 0xFF)
- Compact layout even for large, healthy media

---

### ADR-006: Rolling Adler-32 checksum [2026-04-30]

**Context:**
* To later compare source and destination we need a metric of content.
* Ideally without re-reading both -> checksums!
* It should be computed during the initial traversal.
* Both per-segment and whole-image comparison are worth computing.

**Decision:**
* Implement Adler-32 as a `RollingChecksum` trait
* Trait-based design allows swapping algorithms later
* Maintain two checksum states during `traverse`:
  * Segment-local: reset at each segment boundary, stored as `essence`
  * Layout-global: never reset, stored as `Layout::essence`
* Good segments use checksum value as essence;
* Bad uses sentinel `0xDEADCEEB`;
* Ugly uses the repeating pattern itself

**Consequences:**
+ Global checksum enables fast "are these identical?" check
+ Per-segment essences enable precise mismatch location
+ Adler-32 is fast, non-cryptographic, but adequate for accidental corruption detection
- Chosen over Fletcher-32 for slightly better distribution
- wrapping arithmetic used instead of modulo 65521 for speed

---

### ADR-007: `traverse` as single central loop [2026-05-01]

**Context:**
* The original design had separate functions for scan vs. copy.
* This duplicating the read-and-classify logic.
* Recovery would add a third variant.

**Decision:**
* Unify all traversal into a single `traverse` function 
* Parameterize it by `dst: Option<&mut File>`:
  * `None` → scan-only (Inspect, Compare source/destination)
  * `Some(writer)` → copy mode
* The destination handle is borrowed mutably inside the function via `&mut *dst`
* It is reborrowed at each write call site

**Consequences:**
+ One loop to test, debug, and optimize
* Mode-specific behavior isolated to the `match status` arms
* Recovery pass will reuse `traverse` with a smaller stride and explicit `limit`
+ Avoids callback complexity while keeping the function manageable

---


## Session log
+ **2026-04-02^04:** Initial CLI skeleton:
  + power-of-two parsing;
  + field testing on Windows
+ **2026-04-26:** Redesign to three-pass architecture:
  + `--inspect` mode added;
  + `--limit-size` introduced (later removed).
+ **2026-04-30:** Deep-dive session:
  + heuristic EOF refined;
  + truncation semantics fixed (one-chunk hint);
  + 4-byte Ugly detection designed.
+ **2026-05-01^02:** "The Traversal Engine" major overhaul:
  + `RollingChecksum` trait;
  + `traverse` unification;
  + `PatternRun` deferred writes;
  + Adler-32 implementation;
  + CSV format standardized;
  + `DESIGN.md` created.
