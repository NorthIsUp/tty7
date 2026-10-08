# the traffic lights sit as far in from the left as from the top

`Fork-Feature: traffic-light-inset`. The commit that carries this feature is `fork(traffic-light-inset): …` on `main-niu`.

Upstream put the lights 9pt in, a plain title bar's inset, which crowds them
into the corner of the 48pt bar. AppKit's toolbar windows inset them as far
from the left as from the top, so the fork does the same.

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `src/ui/theme.rs` | `traffic_light_position`, its test | `x` is the derived top gap instead of 9 |
