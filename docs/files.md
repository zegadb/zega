# Files

A file is a node: facts about the file live in the graph, its bytes live in a
**bundle** beside the graph, and a `String<blake3>` field ties them together
(APS 34). zega never stores file bytes inside the graph.

## The standard `File` type

Every intake includes this type:

```zql
type File {
  name: String
  mediaType: String
  size: Int
  hash: String<blake3>
  path?: String<file>
  width?: Int
  height?: Int
  duration?: Float
  source?: String<url>
  licence: String
  author?: String
  credit?: String
  fetchedAt?: String
}
```

- `hash` is the file's blake3 hash, 64 lowercase hex characters. It is the
  asset's name in the bundle, so the graph and the bytes cannot drift apart
  without `verify` failing.
- `path` is where the bytes live on this computer (a `String<file>`, APS 34
  amendment). Desktop indexing writes path + hash and **never copies** the
  bytes into `assets/`.
- `source` is where the file was taken from (a `String<url>`); `licence`,
  `author` and `credit` carry attribution (APS 17).
- `width`, `height` and `duration` describe images, video and audio.

## `String<blake3>`

A string unit checked on every write, exactly like `String<url>` (APS 8). The
value is the blake3 hash of the file's bytes, as 64 lowercase hex characters —
what `b3sum` prints. A bad value is a checker or write error and the whole
write is rolled back:

```zql error
mutation { File(name: "x.png" && mediaType: "image/png" && size: 1 && hash: "x.png" && licence: "CC0") }
```

```text
execution error: error: File.hash must be String<blake3>
  query:1:71
  mutation { File(name: "x.png" && mediaType: "image/png" && size: 1 && hash: "x.png" && licence: "CC0") }
                                                                        ^^^^
  help: write the file's blake3 hash as 64 lowercase hex characters, e.g. `af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262`
```

`String<blake3>` indexes and filters as text, so `hash startsExact "af13"`
works.

## `String<file>`: a local path

A `String<file>` is an absolute `file://` URL with no host, normalised and
percent-encoded, e.g. `file:///Users/ava/Photos/scan.png` (APS 34 amendment).
It is checked on every write like the other string units, and `String<url>`
stays http(s)-only — it still rejects `file://`.

A File's bytes resolve in this order:

1. the local `path`, if it is readable and its bytes still hash to `hash`;
2. then `assets/<hash>`;
3. then the remote in `assets/zega.json`.

A hash mismatch means the file changed on disk: it is **stale** — reported by
`verify` and never resolved, so the wrong image is never shown. Re-index it.
Engines that cannot read the local disk (the browser, zega.earth, cloud)
treat a `String<file>` as absent and fall through to the asset or the remote.

## `@image` over url, file or blake3

`@image: &field` accepts a `String<url>`, `String<file>` or `String<blake3>`
field. A URL points at the image directly; a file or blake3 value resolves by
the order above:

```zql
type Photo {
  name: String
  shot: String<blake3>
}

display {
  graph { Photo(@image: &shot) }
}
```

Any other field type is a checker error.

## `.zga`: the bundle on disk

A bundle is a directory:

```text
name.zga/
  graph.graph        the standard .graph file (docs/graph-format.md)
  assets/<blake3>    the bytes, content-addressed; the name is the hash
  assets/zega.json   optional: where assets not present locally live
```

Assets are stored flat — the hash is the whole name, so a bundle's assets map
one-to-one onto object-storage keys. `zega bundle add` writes an asset
atomically (a temp file, then a rename) and deduplicates: the same bytes twice
are stored once.

`assets/zega.json` records remote locations for assets that are not present
locally — never credentials:

```json
{
  "remote": {
    "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262": "https://static.example.com/photo.jpg"
  }
}
```

`zega bundle verify` fails when:

- an asset's bytes don't hash to its name;
- a `String<blake3>` value in the graph has no matching local path, no local
  asset and no remote entry — a hash counts as present if its local path
  exists and matches, its asset exists, or a remote is listed;
- or any file sits outside the layout.

A local path whose bytes no longer hash to its reference is reported as
stale, not counted.

When the graph carries its schema (the standard case — `zega export
--schema`), exactly the fields declared `String<blake3>` are checked. Without
a schema, every string shaped like a blake3 hash is treated as a reference.

## `.zgz`: the shipping format

A `.zgz` is a gzipped tar of the bundle, and it is **deterministic**: entries
sorted by name, mtime 0, uid and gid 0, modes 644/755, and a gzip header with
no mtime and no filename. The same bundle always packs to the same bytes, so
packs can be cached and compared by hash.

A `.zgz` never ships local paths (APS 34 amendment): `String<file>` values
are stripped from the packed graph — they reveal usernames and folder layout.
A reference whose bytes exist only at a local path refuses to pack;
`zega bundle pack --include-local` copies those bytes into `assets/` first.
Either way the archive contains no `file://` value.

Unpack is safe by construction: it refuses path traversal (`..`), absolute
paths, symlinks, links and device files, writes into a fresh directory, and
runs `verify` before it reports success — a partial or invalid bundle is
removed.

Serve a `.zgz` over HTTP as a file (`application/gzip`), never with
`Content-Encoding: gzip`: the receiver saves the exact bytes.

## CLI

```text
zega bundle new photos.zga
zega bundle add photos.zga photo.jpg        # prints the asset's blake3 hash
zega bundle verify photos.zga
zega bundle pack photos.zga [photos.zgz]    # default: <dir>.zgz
zega bundle pack photos.zga --include-local # copy local file:// bytes into assets/ first
zega bundle unpack photos.zgz photos.zga
```

`add` only stores the bytes; the graph still needs a `File` node whose `hash`
is the printed value. A typical intake is: `new`, then per file `add` plus a
mutation, then `zega export --schema files.zql photos.zga/graph.graph`, then
`verify` and `pack`.

## Rust API

`zega::bundle` (native targets only):

| call | what it does |
|---|---|
| `Bundle::create(path)` | create an empty bundle (fails if `path` exists) |
| `Bundle::open(path)` | open an existing bundle |
| `bundle.put_asset(path)` / `bundle.put_reader(r)` | store bytes under their hash, atomically, deduplicated; returns the hash |
| `bundle.asset_path(hash)` / `bundle.read_asset(hash)` | find / read an asset; `read_asset` re-hashes before returning |
| `bundle.resolve(hash, path)` | local path → asset → remote; a hash mismatch is `Stale`, never resolved |
| `bundle.verify()` | layout, hashes and references (local paths count); returns counts and stale paths |
| `bundle.pack(out)` / `bundle.pack_with(out, include_local)` | verified, deterministic `.zgz` to any `Write`; never ships a local path — `include_local` copies the bytes into `assets/` first |
| `Bundle::unpack(input, dir)` | safe unpack into a fresh directory, verified |
| `manifest_json(&remote)` | the one `assets/zega.json` shape verify accepts |
