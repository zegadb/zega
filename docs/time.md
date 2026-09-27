# Typed history (APS 24, phase 1)

Declare `points: <Int>` to retain valid-time changes. Untyped fields retain only
their current values. `<Int[]>` is a time-varying list; `<Int>[]` declares
time-varying items. The existing whole-array assignment writes every item;
there is no indexed-item assignment syntax in this phase.

```zql
schema {
  type Team {
    name: String
    points:<Int>
  }
}
```

```zql
mutation at 2024-01-01 {
  Team(name: "Oilers" && points: 10) { name }
}
```

```zql
mutation at 2024-01-15 {
  Team(name = "Oilers") set points: 30 { points }
}
```

```zql
mutation at 2024-01-08 {
  Team(name = "Oilers") set points: 20 { points }
}
```

```zql
{
  Team limit 32 { name points }
} as of 2024-01-10
```

```zql
{
  Team(name = "Oilers") { < points > }
} from 2024-01-01 to 2024-02-01 by week
```

```zql
{
  Team(ever points >= 20 && always points >= 10) limit 32 {
    name
    @firstTime(points >= 20)
    @lastTime(points >= 20)
  }
}
```

Dates accept `YYYY-MM-DD` and `YYYY-MM-DDTHH:MM`, in UTC. The only additional
form is `YYYY-MM-DDTHH:MM:SS`, so plain writes stamped at the current UTC second
can round-trip through date-valued selections and comparisons without losing
precision. Results omit seconds when zero. No offsets or relative dates are
accepted. At the same
timestamp the last write replaces the earlier value. Backfills are inserted
in time order; the inline value remains the latest valid-time entry.

An `as of` read returns the latest value at or before the requested instant,
or null before its first entry. Untyped fields and relationships retain their
present values. Phase 2 relationship history and node lifetimes are rejected
with `not yet: APS 24 phase 2`.

Series are inclusive samples at the `from` instant, then every day (24 hours),
week (7 days), or first day of the next calendar month, through `to`. Each
sample is `{time, value}` and carries the value true at that instant. A series
does not sum or average changes within a bucket.

`@firstTime` and `@lastTime` return the first and last recorded change boundary
at which the test is true, or null if it is never true. `ever` and `always`
inspect these recorded states; an empty history satisfies neither. Tests
reuse ordinary Boolean conditions and `has`/`in` chains. Phase 1 chains use
live relationships and historical field values. Scalar ordered comparisons
can skip history scans using min/max summaries.

The live node layout remains unchanged. Histories are a sparse side table;
queries without time never open it. Snapshots carry an optional `HIST` footer,
and `.graph` files an optional checksummed `HIST` section before `DONE`.
The payload is decoded only when history is first needed. Native and wasm
use the same codec. Older files without history still load.

Payload version 1 starts with a string dictionary and a history count. Each
history stores a delta node ID, dictionary field ID, change count, a column
of zigzag delta-of-delta UTC seconds, and a value column. Value tags select
zigzag-delta integers, XOR floats, dictionary strings, null, booleans, or
length-prefixed bincode complex values. Field names and string values share
the dictionary. Summaries are rebuilt when decoding.

Open details beyond phase 1: timezone offsets, subsecond precision, relative
dates, indexed-item assignment syntax, additional series units and
aggregations, future scheduling, windowed predicates, retention, and phase 2
relationship/lifetime semantics. `@lastTime` reports a recorded boundary,
not an inferred end of a continuously true interval.
