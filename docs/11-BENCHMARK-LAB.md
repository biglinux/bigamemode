# Benchmark Lab — method

How BiGame-mode decides whether a change to the machine made a game faster.
This document describes the method; `docs/12-FINDINGS.md` reports what the
method found.

## The problem this method exists to solve

A frame rate is noisy. The same machine running the same workload twice will
not produce the same number, and the gap between two such runs is often larger
than the gap a real optimisation produces. That makes it trivially easy — and
extremely common — to run a benchmark once before a change and once after, see
a higher number, and announce an improvement that is nothing but the spread of
the measurement.

Every decision below exists to make that mistake impossible rather than merely
discouraged.

## Runs alternate; they are not grouped

Arms are interleaved — `A B A B A B` — never grouped as `A A A B B B`.

Grouping confounds the configuration with anything that changes over the course
of the session. The dominant such thing is chassis temperature: a GPU that has
been rendering for ten minutes boosts lower than one that started cold, so
whichever arm runs last is penalised by however long the session has been
going. This is not hypothetical. In the isolation matrix below, the overall
level fell from about 720 fps to about 650 fps over roughly twenty minutes of
continuous benchmarking — a 10 % drift, far larger than any difference the
experiment was trying to detect. Alternating spreads that drift across both
arms instead of assigning it to one.

## The first run of every arm is discarded

A cold shader cache and a cold GPU make the first run unlike every run that
follows it. Its number is thrown away rather than averaged in.

This also means cold-cache and warm-cache behaviour are separate questions, and
a benchmark that wants to measure shader compilation stutter has to be built to
do that deliberately — it cannot be read out of a warm-cache run.

## A difference must pass two tests to be called real

Both, not either:

1. **It must exceed the run-to-run spread of both arms.** A difference smaller
   than the variation within an arm cannot be attributed to the change; it is
   indistinguishable from the noise the arm already exhibits.

2. **It must survive Welch's t-test at 95 %.** Welch's rather than Student's
   because the arms have no reason to share a variance: a configuration that
   raises a frame rate often changes its consistency too, and assuming equal
   variances would credit the result with confidence it has not earned.

Anything that fails either test is reported as **no change**, and the
percentage is withheld from the comparison column entirely. The raw number
remains in the JSON, because it is a fact; what it *means* is what the verdict
says, and the two must not be separated.

## An arm that disagrees with itself is refused

If the runs within a single arm vary by more than 5 %, the comparison is
reported as **inconclusive** rather than averaged. A spread that large means
something outside the experiment was running during it, and no arithmetic
applied to those numbers recovers a trustworthy answer.

## A capped workload is refused outright

SuperTuxKart ships with vsync on and `max_fps=120`, which pins it near 160 fps
on this hardware. A capped workload cannot show a difference between two
configurations however large that difference is — both arms simply hit the cap.

Running a comparison anyway would produce a confident-looking "no change" that
means nothing, which is worse than no comparison at all, because it looks like
evidence. The provider therefore reports the cap as a dependency problem and
declines to run.

With the cap lifted the same 38-second replay went from 6 101 frames to 27 871
— 4.5× the work — which is what makes it able to detect anything.

## What is measured, and what is only sanity-checked

`glxgears` and `vkcube` are **sanity checks**, never evidence. They confirm
that a driver stack is alive. They are not games, they do not exercise a game's
CPU/GPU balance, and a number from either says nothing about gaming
performance.

The measured workloads are ranked by how much they resemble the thing being
optimised for, and by whether they can be driven without a person in the chair.

## Telemetry accompanies every run

GPU clock, memory clock, power draw, temperature, utilisation and VRAM are
sampled four times a second for the duration of each run. A frame rate alone
cannot explain itself; when one arm is slower, the clocks are where the reason
is.

## Nothing is fabricated

A benchmark that cannot be run is recorded as **NOT TESTED**, with the reason.
A metric the hardware does not expose is recorded as **NOT AVAILABLE**. Neither
is quietly omitted, because an absent row reads as an oversight while an
explicit one reads as a fact — and because a reader who cannot tell the
difference between "we measured nothing" and "we measured nothing worth
reporting" cannot trust any of it.

## The result layout

Every session writes one directory with a fixed shape, so that a result from a
year ago is readable by the same code as one from today:

```
benchmarks/<date>-<workload>/
  system.json        the machine, and a fingerprint of it
  benchmark.json     what was run, how many runs, whether alternated
  <arm>/run-NN/      raw artifacts and telemetry, one directory per run
  comparison.json    the verdicts
  comparison.csv     the same, for a spreadsheet
  report.md          the same, for a person
```

`system.json` carries no hostname, username, home directory or IP address. A
test asserts it, so the file is safe to attach to a shared report without a
sanitising step that someone has to remember to perform.

## The machine is driven through the product, not around it

The harness changes the machine by calling BiGame-mode's own D-Bus daemon —
the same path the Booster button uses — rather than by writing to sysfs
directly. What is measured is therefore the Booster as shipped, not a shell
approximation of it that might differ from it in some detail.

The machine's original state is captured before anything changes and restored
on exit, including on interrupt.
