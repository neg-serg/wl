# Data Model: Creative Transition Shaders

## Entity Changes

### TransitionType (common/src/ipc_types.rs)

IPC-level enum serialized between client and daemon.

| Variant   | Existing | Notes                          |
|-----------|----------|--------------------------------|
| Fade      | yes      |                                |
| Wipe      | yes      |                                |
| Grow      | yes      |                                |
| Wave      | yes      |                                |
| Outer     | yes      |                                |
| Pixelate  | **new**  |                                |
| Swirl     | **new**  |                                |
| Blinds    | **new**  |                                |
| Diamond   | **new**  |                                |
| Dissolve  | **new**  |                                |
| Random    | yes      | Now selects from 10 types      |
| None      | yes      |                                |

### TransitionKind (daemon/src/vulkan/pipeline.rs)

Internal enum (excludes None and Random). Used as HashMap key for pipeline lookup.

| Variant   | Existing | Shader file              |
|-----------|----------|--------------------------|
| Fade      | yes      | transition_fade.frag     |
| Wipe      | yes      | transition_wipe.frag     |
| Grow      | yes      | transition_grow.frag     |
| Wave      | yes      | transition_wave.frag     |
| Outer     | yes      | transition_outer.frag    |
| Pixelate  | **new**  | transition_pixelate.frag |
| Swirl     | **new**  | transition_swirl.frag    |
| Blinds    | **new**  | transition_blinds.frag   |
| Diamond   | **new**  | transition_diamond.frag  |
| Dissolve  | **new**  | transition_dissolve.frag |

### TransitionTypeArg (client/src/cli.rs)

CLI argument enum with clap `value_enum` derive.

| Variant   | CLI value   | Maps to TransitionType |
|-----------|-------------|------------------------|
| Pixelate  | `pixelate`  | Pixelate               |
| Swirl     | `swirl`     | Swirl                  |
| Blinds    | `blinds`    | Blinds                 |
| Diamond   | `diamond`   | Diamond                |
| Dissolve  | `dissolve`  | Dissolve               |

## Relationships

```
TransitionTypeArg (CLI) --[From impl]--> TransitionType (IPC)
TransitionType (IPC) --[resolve_kind]--> TransitionKind (daemon)
TransitionKind --[HashMap key]--> vk::Pipeline (Vulkan)
TransitionKind <--[shader file]--> .frag GLSL source
```

## Push Constants (unchanged)

`TransitionPushConstants` layout shared by all transition shaders:

| Field            | Type  | Usage by new shaders                        |
|------------------|-------|---------------------------------------------|
| progress         | f32   | All: animation progress 0.0→1.0             |
| angle            | f32   | Blinds: orientation; Swirl: direction        |
| pos_x            | f32   | Swirl/Diamond: center X                      |
| pos_y            | f32   | Swirl/Diamond: center Y                      |
| wave_x           | f32   | Blinds: strip count; Swirl: strength         |
| wave_y           | f32   | Reserved                                     |
| old_resize_mode  | u32   | All: UV transform for old texture            |
| old_img_aspect   | f32   | All: aspect ratio of old texture             |
| new_resize_mode  | u32   | All: UV transform for new texture            |
| new_img_aspect   | f32   | All: aspect ratio of new texture             |
| screen_aspect    | f32   | All: screen aspect ratio                     |
