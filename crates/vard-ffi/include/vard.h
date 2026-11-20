/*
 * vard.h — C ABI of the Vard runtime (crates/vard-ffi) and the GPU renderer (crates/vard-gpu).
 *
 * All calls happen on the UI thread and never from inside a VardNativeFn callback.
 * Coordinates and sizes are dp; event calls return 1 (handled), 0 (nothing reacted)
 * or -1 (error / not mounted). Strings are NUL-terminated UTF-8.
 * Kept in sync with the Rust sources by crates/vard-ffi/tests/header.rs.
 */
#ifndef VARD_H
#define VARD_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct VardApp VardApp;
typedef struct VardGpu VardGpu;

typedef struct VardSource {
    const char *path;
    const char *text;
} VardSource;

/* Answer `extern fn name(args_json)`: a JSON string valid until the next call, or NULL
 * when the function is not implemented. Function arguments arrive as {"$callback": id}. */
typedef const char *(*VardNativeFn)(void *ctx, const char *name, const char *args_json);

/* ---- lifecycle ---- */
VardApp *vard_create(const VardSource *files, size_t count, float width, float height);
void vard_destroy(VardApp *app);
void vard_set_host(VardApp *app, VardNativeFn f, void *ctx, const char *platform);
int32_t vard_add_font(VardApp *app, const char *file_name, const uint8_t *bytes, size_t len);
void vard_set_asset_dir(VardApp *app, const char *dir);
int32_t vard_mount(VardApp *app, const char *view);
const char *vard_diagnostics(VardApp *app);

/* ---- events ---- */
int32_t vard_tap(VardApp *app, float x, float y);
int32_t vard_input_text(VardApp *app, uint32_t uid, const char *text);
int32_t vard_submit(VardApp *app, uint32_t uid);
void vard_focus(VardApp *app, uint32_t uid);
int32_t vard_scroll(VardApp *app, float x, float y, float dy);
void vard_set_dark(VardApp *app, bool dark);
void vard_set_locale(VardApp *app, const char *tag);
void vard_resize(VardApp *app, float width, float height);
void vard_set_insets(VardApp *app, float top, float bottom, float left, float right, float keyboard);
int32_t vard_emit(VardApp *app, const char *name, const char *args_json);
int32_t vard_callback(VardApp *app, uint64_t id, const char *json, bool keep);
void vard_release(VardApp *app, uint64_t id);
bool vard_back_enabled(VardApp *app);
void vard_back_progress(VardApp *app, float progress);
void vard_back_cancel(VardApp *app);
int32_t vard_back(VardApp *app);

/* ---- frames and animation ---- */
/* Advance `animate:`/`enter:` animations by dt_ms since the previous vsync; true while running. */
bool vard_advance(VardApp *app, float dt_ms);
/* The laid-out screen (format: crates/vard-ffi/src/frame.rs), owned by app until the next call. */
const uint8_t *vard_frame(VardApp *app, size_t *out_len);

/* ---- GPU renderer (crates/vard-gpu): draws frames into a CAMetalLayer ---- */
/* layer: a CAMetalLayer*; width/height in pixels; scale = pixels per dp. NULL if no GPU. */
VardGpu *vard_gpu_create_metal(void *layer, uint32_t width, uint32_t height, float scale);
void vard_gpu_resize(VardGpu *gpu, uint32_t width, uint32_t height, float scale);
/* Draw the app's current frame; returns true while animations run (keep the display link on). */
bool vard_gpu_render(VardGpu *gpu, VardApp *app);
void vard_gpu_destroy(VardGpu *gpu);

/* ---- OTA and on-device inspection ---- */
/* Verify and install an OTA bundle; returns NULL on success or an error string to free. */
char *vard_ota_install(const char *latest, const uint8_t *bundle, size_t len, const char *key, const char *fingerprint,
                       const char *dir);
void vard_string_free(char *s);
int32_t vard_inspect_listen(VardApp *app, const char *addr);
int32_t vard_inspect_poll(VardApp *app);

#ifdef __cplusplus
}
#endif

#endif /* VARD_H */
