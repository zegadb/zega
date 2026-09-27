# Typed history (APS 24, phases 1 and 2)

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
or null before its first entry. Untyped fields retain their present values.
Time-typed relationships use half-open validity intervals `[from, to)`; an
ended relationship is absent at `to`. Nodes are visible from `appears at`
and invisible at `ends at`. Untyped relationships retain their present
membership, but a temporal traversal still filters their endpoints by lifetime.

Series are inclusive samples at the `from` instant, then every day (24 hours),
week (7 days), or first day of the next calendar month, through `to`. Each
sample is `{time, value}` and carries the value true at that instant. A series
does not sum or average changes within a bucket.

`@firstTime` and `@lastTime` return the first and last recorded change boundary
at which the test is true, or null if it is never true. `ever` and `always`
inspect these recorded states; an empty history satisfies neither. Tests
reuse ordinary Boolean conditions and `has`/`in` chains, with historical
relationships, values and lifetimes evaluated at the same instant. Temporal
tests also accept nested `has`, as in the APS 24 film/genre example. Ordinary
filters keep their existing flat-chain grammar. Scalar ordered comparisons
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

Open details beyond phase 2: timezone offsets, subsecond precision, relative
dates, indexed-item assignment syntax, additional series units and
aggregations, future scheduling, and retention. `@lastTime` reports a recorded
boundary, not an inferred end of a continuously true interval.

## Rosters and lifetimes

Wrap the relationship target in `<…>` to keep membership history. Inverse
fields can name the same stored kind. Lifetime declarations reference `Date`
fields; an absent optional bound is unbounded. Each declared lifetime bound
adds one stored date per node, without changing the live node record.

```zql
schema {
  type Team {
    name: String
    players -> <Person[]>
  }

  type Person {
    name: String
    born: Date
    died?: Date
    team: players <- <Team[]>
    children -> Person[]
    appears at born
    ends at died
  }
}
```

```zql
mutation at 2023-10-10 {
  Team(name: "Oilers") {
    players -> Person(name: "Pat" && born: "1990-01-01") { name }
  }
}
```

```zql
mutation at 2024-02-01 {
  Team(name = "Oilers") {
    players -> unlink Person(name = "Pat") { name }
  }
}
```

`unlink` finds existing nodes, removes their live relationship, and retains
its interval only when its kind is time-typed. The existing `link` form adds
it again as a new interval. Plain writes use now; `mutation at` supplies valid
time. An end before its relationship's start fails atomically. Removing a
node explicitly purges its history and incident relationship histories;
use `ends at` to retain a node that ceased to exist at a date.

```zql
{
  Team(name = "Oilers") {
    players -> Person { name }
  }
} as of 2024-01-15
```

```zql
{
  Team(name = "Oilers") {
    players -> Person during 2023-10-10 to 2024-04-18 { name }
  }
}
```

`during` returns each matching node once if the complete test holds at any
instant in the inclusive window. Its projected row comes from the first
matching state. All hops and tests use the same instant, so relationships
that existed at disjoint times cannot form a fictitious path.

```zql
{
  Team(name = "Oilers") {
    players -> Person changes from 2024-01-01 to 2024-03-08 { name }
  }
}
```

`changes` compares the two endpoint states and returns `{joined, left,
changed}`. Joined/left arrays contain projected rows. A changed row has
`{id, from, to}` with the two projections. Identity determines membership;
an end and re-add between the endpoints cancels out if membership and
selected values are unchanged. This is a state difference, not an event log.
Both window forms also work after the outer query block to compare values.

```zql
{
  Person limit 100 { name @firstTime(has team(name = "Oilers")) }
}
```

```zql
{
  Person(always has team(name = "Oilers") during 2023-11-01 to 2024-01-01) limit 100 {
    name
  }
}
```

## Named periods: seasons are nodes

A bare `2026` means the calendar year: January 1 in `as of`, and January 1
through December 31 in `during 2026`. Declaring a calendar never changes that
meaning. Name a season explicitly with `season 26`, `season 2026`, or
`season 26-27`; all three select the node whose Int name is 2026. Two-digit
years mean 20xx, and a pair must be consecutive.

The schema declares the date and name fields, then gives each consuming type
one or more calendar words. The dates themselves live on ordinary nodes.

```zql
schema {
  type Season {
    year: Int
    starts: Date
    ends: Date
    games:<Int>
    period from starts to ends named by year
  }

  type Team {
    name: String
    points:<Int>
    calendar season -> Season
  }
}
```

The NHL's 2020 COVID season ran in 2021. Its actual dates take precedence over
any assumption about the year in its name:

```zql
mutation {
  Season {
    year: 2020
    starts: 2021-01-13
    ends: 2021-07-07
    games: 56
  }
}
```

The NHL moved from 82 to 84 games per team for 2026–27. Because `games` is
time-typed, the same Season node can retain that change. The announcement date
of July 1, 2025 below is illustrative, as are the creation date and season
endpoints; they are example data, not an official schedule.

```zql
mutation at 2025-01-01 {
  Season {
    year: 2026
    starts: 2026-10-01
    ends: 2027-06-30
    games: 82
  }
}
```

```zql
mutation at 2025-07-01 {
  Season(year: 2026) set games: 84
}
```

Before the announcement, the recorded schedule still had 82 games:

```zql
{
  Season(year = 2026) { games }
} as of 2025-06-30
```

```json
{ "games": 82 }
```

The current value is 84:

```zql
{
  Season(year = 2026) { games }
}
```

```json
{ "games": 84 }
```

The first recorded date with 84 games is the illustrative announcement date:

```zql
{
  Season(year = 2026) { @firstTime(games = 84) }
}
```

```json
{ "firstTime": "2025-07-01T00:00" }
```

For an illustrative points history:

```zql
mutation at 2021-01-13 {
  Team(name: "Example" && points: 50) { name }
}
```

```zql
mutation at 2021-07-07 {
  Team(name = "Example") set points: 56 { points }
}
```

```zql
{
  Team(always points >= 50 during season 20) limit 100 { name }
}
```

```zql
{
  Team(ever points >= 50 during season 20) limit 100 { name }
}
```

The lost 2004 NHL season can be omitted. Then `during season 04` reports
`no season 2004 in Season` and asks for a node with its dates; it never guesses
boundaries. Alternatively, keep a Season node with `games: 0` and the dates
of the planned span if your dataset records the cancelled schedule. A period
with zero games still defines a window; games are ordinary data.

For a cross-year season, store its real endpoints:

```zql
mutation {
  Season {
    year: 2023
    starts: 2023-10-10
    ends: 2024-06-24
    games: 82
  }
}
```

```zql
{
  Team limit 100 { name points }
} during season 23
```

```zql
{
  Team limit 100 { name points }
} as of start of season 23
```

```zql
{
  Team limit 100 { name points }
} as of end of season 23
```

The series uses the period nodes already in this example:

```zql
{
  Team limit 100 { name < points > }
} from season 20 to season 23 by season
```

Once season 25 is present, `as of end of season 25` uses its stored end.

`from season 24 to season 26` uses the first node's start and the last node's
end. `by season` produces one sample per period node overlapping that range,
ordered by its stored start date, with the field's value at that start. It
does not invent buckets for absent seasons or aggregate values. Endpoints
are the stored Date instants, with the same inclusive window semantics as
explicit dates. Named dates also work in `changes from … to …` and time
function comparisons.

Other period types use their naming field's value:

```zql
schema {
  type Era {
    name: String
    starts: Date
    ends: Date
    period from starts to ends named by name
  }

  type Team {
    name: String
    points:<Int>
    calendar era -> Era
  }
}
```

This example dataset defines its era using the calendar years 1942–1967:

```zql
mutation {
  Era {
    name: "Original Six"
    starts: 1942-01-01
    ends: 1967-12-31
  }
}
```

```zql
{
  Team limit 100 { name }
} during era "Original Six"
```

A type may declare both `calendar season -> Season` and `calendar era -> Era`.
Each word resolves on the type where it is used. Missing calendars, missing
period declarations, unknown names, and duplicate names report how to fix
the schema or data. Start and end fields must exist and be Date; the naming
field must exist. Period nodes need valid dates with start no later than end.

Adding or correcting a period uses an ordinary mutation. Queries resolve
against the current graph, including when the query reads historical facts;
adding next season requires no schema change. Adding a period declaration or
calendar is safe in schema diff; removing or renaming a calendar word warns.

Adding relationship history typing or a lifetime declaration is safe in
schema diff. Removing history typing from a relationship with retained
intervals is blocking, including when every relationship has ended.

HIST payload version 2 embeds the unchanged version-1 field columns, then
relationship columns (delta IDs, a kind dictionary, delta endpoint IDs,
delta-of-delta start dates, optional end offsets, and edge properties) and
lifetime columns (delta node IDs and delta-of-delta dates). Field-only files
still use version 1. Old files and phase-1 files load without migration, and
all history stays lazy until a time read or temporal write needs it.

Reads without time use the original live adjacency and never open history.
Temporal relationship reads consult the side store; window and relationship
tests evaluate relevant change boundaries. No copies of the whole graph are
stored. Retention and segmented on-disk history remain phase 3.
