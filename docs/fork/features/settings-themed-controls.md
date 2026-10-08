# Settings' controls take the theme's colour

`Fork-Feature: settings-themed-controls`. The commit that carries this feature is `fork(settings-themed-controls): …` on `main-niu`.

On a light theme Settings drew its dropdowns, buttons, menus, chips and switch
knobs pure white, which stood out on a tinted page such as a cream theme. They are
now the page colour lifted a step, opaque, so they keep the theme's hue: a white
theme still gets white controls. Dark themes are unchanged.

## Hooks in upstream files

| file | function / site | why |
|---|---|---|
| `src/ui/settings/kit.rs` | `Tk::of`: `btn`, `menu`, `knob`, `chip` | light controls from `lift(page)` instead of white |
