# Stopping a preview

A consumer that shows previews under a time limit calls `preview` with a
`keep_going` callback. brushkit calls it before it builds each entry and stops
at the first `false`. The result holds the entries built so far, and
`not_reached` counts the entries that were not built. The work before the
first entry, such as opening the zip, is not interrupted.

## Sub-features

- `stop-entries` returns the entries built before the first `false`, as `take` selects them.
- `stop-not-reached` counts the entries not built, and is `0` for a call that was not stopped.
- `stop-calls` calls `keep_going` once per entry built plus once for the `false`, and not again after `take` has all it needs.
- `stop-take` combines with `Take::FirstAvailable(n)`, whose unavailable entries also count as built.

## How to get to it (user POV)

- `brushkit::preview::preview(&bytes, Format::Abr | Format::Brush | Format::Brushset, opts, Take::All | Take::FirstAvailable(n), &mut keep_going)`.
- `PreviewSet::not_reached`.

## Driving it with verify-brushkit

Preconditions:

- Baseline preconditions from the index hold.
- `--stop-after K` makes the probe call `preview` with a `keep_going` that returns `true` K times. The call head reports `stop_after` and `keep_going_calls`. Without the flag the probe calls the six wrappers and `not_reached` is `0`.

- **Stop part way.** Run `$H run stop2 preview fuzz/corpus/preview_abr/unavailable_tips --kind abr --stop-after 2`. Exit `0`. The names are `["B","A"]`, `.result.not_reached` is `2` and `.result.call.keep_going_calls` is `3`.
- **Stop at once.** The same seed with `--stop-after 0` has no entries, `not_reached` `4` and `keep_going_calls` `1`.
- **Not stopped.** The same seed with `--stop-after 4` has 4 entries, `not_reached` `0` and `keep_going_calls` `4`, because `keep_going` is not asked about an entry that does not exist.
- **First available.** The same seed with `--first 1 --stop-after 100` returns `A` at index 1 with `not_reached` `0` and `keep_going_calls` `2`. With `--first 2 --stop-after 1` it returns no entries, `not_reached` `3`, because the unavailable `B` was built first.
- **Set and brush.** Run `$H run stop-set preview fuzz/corpus/preview_brushset/ordered_set --kind brushset --stop-after 1`. One entry `B`, `set_name` `"Ordered"`, `not_reached` `1`. `fuzz/corpus/preview_brush/root_brush --kind brush --stop-after 0` has no entries and `not_reached` `1`.
- **Real pack.** Run `$H run stop5 preview "$BRUSHKIT_CORPUS_DIR/<pack>.abr" --stop-after 5` and a full run of the same pack. `stop5` has 5 entries equal to the first 5 of the full run, and `not_reached` is `counts.entries` of the full run minus 5.

## Gotchas

- The probe stops on a count, not a clock. That proves where the call stops, not how long it takes. The time an `.abr` index parse or a zip open takes is spent before the first `keep_going` call.
- A stopped call is still `Ok`, exit `0`. Only `not_reached` tells it from a complete one.
