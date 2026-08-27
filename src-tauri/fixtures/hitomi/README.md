# Hitomi source contract fixtures

These fixtures are synthetic and deterministic. They reproduce the field and
wrapper shapes observed in the upstream metadata protocol without retaining a
live gallery title, tag set, image hash, or image payload.

- `galleryinfo-normal.js` mirrors the current `var galleryinfo = {...};` wrapper.
  Its first file deliberately omits `haswebp`, as current payloads may do.
- `gg-current.js` is a compact representation of the current classic `gg.m`,
  `gg.s`, and `gg.b` shape. The route cases are synthetic.
- `nozomi-range.hex` represents four big-endian unsigned gallery IDs.
- HTTP and transport policy fixtures contain no response bodies.

The fixtures must stay local-only tests. They are not permission to fetch live
gallery images during the test suite.
