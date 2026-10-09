// Real SHM-backed Wayland clients for vm-layer-smoke.py; no compositor test
// hooks.
// animate NAME drives changing pixels from frame callbacks, respecting
// suspension and output visibility.
// pattern NAME redraws a high-frequency patch for preview resolution checks.
// alpha NAME N redraws a mapped, configured view with alpha 0..255 (default 255).
// subsurface NAME PARENT W H X Y COLOR creates a desynchronized child above its
// parent, at parent buffer coordinates (not XDG geometry coordinates). Creation
// commits the parent to apply the position; later child commits are independent.
// app-xdg/app-kde NAME W H FIXED OFFSET COLOR MODE negotiates before the first
// bufferless commit. MODE is default/server/client (KDE also none).
// app-xdg-deferred has the same arguments but waits for start NAME before its
// first bufferless commit, allowing configure deferral to be observed.
// decorate NAME xdg|kde MODE changes/creates negotiation; xdg also unset, both
// destroy. Recreating XDG v1 requires an unmapped surface, before remap NAME.
// hold NAME ACKs XDG configures without drawing; commit NAME releases the hold.
// remap NAME performs a fresh bufferless XDG handshake after unmap NAME.
// title NAME TEXT changes the title (omitting TEXT clears it).
// popup NAME PARENT X Y W H COLOR accepts layer or XDG toplevel parents.
#define _GNU_SOURCE
#include "server-decoration-client-protocol.h"
#include "wlr-layer-shell-client-protocol.h"
#include "xdg-decoration-client-protocol.h"
#include "xdg-shell-client-protocol.h"
#include <errno.h>
#include <poll.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>
#include <wayland-client.h>

struct view {
    char name[32];
    struct wl_surface *surface;
    struct wl_subsurface *subsurface;
    struct view *parent;
    struct xdg_surface *xdg;
    struct xdg_toplevel *top;
    struct zxdg_toplevel_decoration_v1 *xdg_decoration;
    struct org_kde_kwin_server_decoration *kde_decoration;
    uint32_t xdg_mode, kde_mode, ack_serial;
    unsigned xdg_decoration_count, kde_decoration_count;
    struct xdg_popup *popup;
    struct zwlr_layer_surface_v1 *layer;
    int width, height, requested_w, requested_h, offset;
    int committed_w, committed_h, configure_w, configure_h, popup_x, popup_y;
    unsigned configure_count, commit_count;
    bool hold_commit;
    bool animated, suspended, sharp_pattern;
    unsigned frame_count;
    int outputs;
    struct wl_callback *frame;
    uint32_t color, alpha;
    bool mapped, configured, fixed, maximized;
};
static struct wl_display *display;
static struct wl_compositor *compositor;
static struct wl_subcompositor *subcompositor;
static struct wl_shm *shm;
static struct xdg_wm_base *wm;
static struct zxdg_decoration_manager_v1 *xdg_decoration_manager;
static struct org_kde_kwin_server_decoration_manager *kde_decoration_manager;
static uint32_t kde_default_mode;
static unsigned kde_default_count;
static struct zwlr_layer_shell_v1 *shell;
static struct wl_seat *seat;
static struct wl_keyboard *keyboard;
static struct view views[16];
static int count;
static struct wl_surface *focus;

static void fail(const char *message) {
    fprintf(stderr, "fixture: %s (errno=%d)\n", message, errno);
    exit(1);
}
static struct view *find(const char *name) {
    for (int i = 0; i < count; i++)
        if (!strcmp(views[i].name, name))
            return &views[i];
    fail("unknown view");
    return NULL;
}
static const char *surface_name(struct wl_surface *surface) {
    for (int i = 0; i < count; i++)
        if (views[i].surface && views[i].surface == surface)
            return views[i].name;
    return "none";
}
struct pixels {
    void *data;
    size_t size;
};
static void release(void *data, struct wl_buffer *buffer) {
    struct pixels *pixels = data;
    munmap(pixels->data, pixels->size);
    free(pixels);
    wl_buffer_destroy(buffer);
}
static const struct wl_buffer_listener buffer_listener = {.release = release};
static uint32_t premultiply(uint32_t color, uint32_t alpha) {
    return (alpha << 24) | ((((color >> 16) & 255) * alpha / 255) << 16) |
           ((((color >> 8) & 255) * alpha / 255) << 8) |
           ((color & 255) * alpha / 255);
}
static void draw(struct view *v);
static void frame_done(void *data, struct wl_callback *callback, uint32_t time) {
    (void)time;
    struct view *v = data;
    wl_callback_destroy(callback);
    v->frame = NULL;
    if (v->animated && v->mapped && !v->suspended && v->outputs > 0) {
        v->frame_count++;
        // Encode progress in the red channel for the GPU capture oracle.
        v->color = 0xff000000 | ((16 + (v->frame_count / 4 % 96) * 2) << 16) | 0xb060;
        draw(v);
    }
}
static const struct wl_callback_listener frame_listener = {.done = frame_done};
static void draw(struct view *v) {
    int w = v->fixed ? v->requested_w : v->width;
    int h = v->fixed ? v->requested_h : v->height;
    if (w <= 0 || h <= 0 || w > 4096 || h > 4096)
        fail("invalid buffer size");
    int bw = w + 2 * v->offset, bh = h + 2 * v->offset;
    size_t size = (size_t)bw * bh * 4;
    int fd = memfd_create("clear-layer-smoke", MFD_CLOEXEC);
    if (fd < 0 || ftruncate(fd, (off_t)size))
        fail("memfd");
    struct pixels *pixels = calloc(1, sizeof(*pixels));
    if (!pixels)
        fail("allocation");
    pixels->size = size;
    pixels->data = mmap(NULL, size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (pixels->data == MAP_FAILED)
        fail("mmap");
    uint32_t *p = pixels->data;
    uint32_t color = premultiply(v->color, v->alpha);
    uint32_t margin = premultiply(0xffa030c0, v->alpha);
    // A contrasting buffer margin detects placement using surface rather than
    // XDG geometry.
    for (int y = 0; y < bh; y++)
        for (int x = 0; x < bw; x++)
            p[y * bw + x] = x >= v->offset && x < w + v->offset &&
                                    y >= v->offset && y < h + v->offset
                                ? color
                                : margin;
    if (v->sharp_pattern) {
        for (int y = v->offset + 100; y < v->offset + 180 && y < bh; y++)
            for (int x = v->offset + 100; x < v->offset + 220 && x < bw; x++)
                p[y * bw + x] = premultiply(x % 2 ? 0xffffff : 0, v->alpha);
    }
    struct wl_shm_pool *pool = wl_shm_create_pool(shm, fd, (int)size);
    struct wl_buffer *buffer = wl_shm_pool_create_buffer(
        pool, 0, bw, bh, bw * 4, WL_SHM_FORMAT_ARGB8888);
    wl_buffer_add_listener(buffer, &buffer_listener, pixels);
    wl_shm_pool_destroy(pool);
    close(fd);
    if (v->xdg)
        xdg_surface_set_window_geometry(v->xdg, v->offset, v->offset, w, h);
    wl_surface_attach(v->surface, buffer, 0, 0);
    wl_surface_damage(v->surface, 0, 0, bw, bh);
    if (v->animated && !v->suspended && v->outputs > 0 && !v->frame) {
        v->frame = wl_surface_frame(v->surface);
        wl_callback_add_listener(v->frame, &frame_listener, v);
    }
    wl_surface_commit(v->surface);
    v->committed_w = w;
    v->committed_h = h;
    v->commit_count++;
    printf("{\"event\":\"commit\",\"name\":\"%s\",\"width\":%d,\"height\":%d,"
           "\"offset\":%d}\n",
           v->name, w, h, v->offset);
}
static void xdg_configure(void *data, struct xdg_surface *surface,
                          uint32_t serial) {
    struct view *v = data;
    xdg_surface_ack_configure(surface, serial);
    v->configure_count++;
    v->ack_serial = serial;
    v->configured = true;
    printf("{\"event\":\"xdg-ack\",\"name\":\"%s\",\"serial\":%u,"
           "\"decoration_mode\":%u,\"width\":%d,\"height\":%d,\"held\":%s}\n",
           v->name, serial, v->xdg_mode, v->configure_w, v->configure_h,
           v->hold_commit ? "true" : "false");
    if (v->mapped && !v->hold_commit)
        draw(v);
}
static const struct xdg_surface_listener xdg_listener = {.configure =
                                                             xdg_configure};
static void top_configure(void *data, struct xdg_toplevel *top, int32_t w,
                          int32_t h, struct wl_array *states) {
    (void)top;
    struct view *v = data;
    v->maximized = false;
    v->suspended = false;
    uint32_t *state;
    wl_array_for_each(state, states) {
        if (*state == XDG_TOPLEVEL_STATE_MAXIMIZED)
            v->maximized = true;
        if (*state == XDG_TOPLEVEL_STATE_SUSPENDED)
            v->suspended = true;
    }
    v->configure_w = w;
    v->configure_h = h;
    v->width = w > 0 ? w : v->requested_w;
    v->height = h > 0 ? h : v->requested_h;
    printf("{\"event\":\"configure\",\"name\":\"%s\",\"width\":%d,\"height\":%"
           "d}\n",
           v->name, w, h);
}
static void top_close(void *data, struct xdg_toplevel *top) {
    (void)data;
    (void)top;
    fail("unexpected XDG close");
}
static void top_bounds(void *data, struct xdg_toplevel *top, int32_t w, int32_t h) {
    (void)data;
    (void)top;
    (void)w;
    (void)h;
}
static void top_capabilities(void *data, struct xdg_toplevel *top, struct wl_array *caps) {
    (void)data;
    (void)top;
    (void)caps;
}
static const struct xdg_toplevel_listener top_listener = {
    .configure = top_configure,
    .close = top_close,
    .configure_bounds = top_bounds,
    .wm_capabilities = top_capabilities,
};
static void popup_configure(void *data, struct xdg_popup *popup, int32_t x,
                            int32_t y, int32_t w, int32_t h) {
    (void)popup;
    struct view *v = data;
    v->popup_x = x;
    v->popup_y = y;
    v->width = v->configure_w = w;
    v->height = v->configure_h = h;
    printf("{\"event\":\"popup-configure\",\"name\":\"%s\",\"x\":%d,\"y\":%d,"
           "\"width\":%d,\"height\":%d}\n",
           v->name, x, y, w, h);
}
static void popup_done(void *data, struct xdg_popup *popup) {
    (void)data;
    (void)popup;
    fail("unexpected popup dismissal");
}
static const struct xdg_popup_listener popup_listener = {
    .configure = popup_configure,
    .popup_done = popup_done,
};
static void layer_configure(void *data, struct zwlr_layer_surface_v1 *layer,
                            uint32_t serial, uint32_t w, uint32_t h) {
    struct view *v = data;
    zwlr_layer_surface_v1_ack_configure(layer, serial);
    v->width = v->configure_w = (int)w;
    v->height = v->configure_h = (int)h;
    v->configure_count++;
    v->configured = true;
    printf("{\"event\":\"configure\",\"name\":\"%s\",\"width\":%u,\"height\":%"
           "u}\n",
           v->name, w, h);
    if (v->mapped && !v->hold_commit)
        draw(v);
}
static void layer_closed(void *data, struct zwlr_layer_surface_v1 *layer) {
    (void)data;
    (void)layer;
    fail("unexpected layer close");
}
static const struct zwlr_layer_surface_v1_listener layer_listener = {
    .configure = layer_configure,
    .closed = layer_closed,
};
static void ping(void *data, struct xdg_wm_base *base, uint32_t serial) {
    (void)data;
    xdg_wm_base_pong(base, serial);
}
static const struct xdg_wm_base_listener wm_listener = {.ping = ping};
static void keymap(void *data, struct wl_keyboard *kb, uint32_t format, int fd,
                   uint32_t size) {
    (void)data;
    (void)kb;
    (void)format;
    (void)size;
    close(fd);
}
static void enter(void *data, struct wl_keyboard *kb, uint32_t serial,
                  struct wl_surface *surface, struct wl_array *keys) {
    (void)data;
    (void)kb;
    (void)serial;
    (void)keys;
    focus = surface;
    printf("{\"event\":\"enter\",\"name\":\"%s\"}\n", surface_name(surface));
}
static void leave(void *data, struct wl_keyboard *kb, uint32_t serial,
                  struct wl_surface *surface) {
    (void)data;
    (void)kb;
    (void)serial;
    if (focus == surface)
        focus = NULL;
    printf("{\"event\":\"leave\",\"name\":\"%s\"}\n", surface_name(surface));
}
static void key(void *data, struct wl_keyboard *kb, uint32_t serial,
                uint32_t time, uint32_t code, uint32_t state) {
    (void)data;
    (void)kb;
    (void)serial;
    (void)time;
    (void)code;
    (void)state;
}
static void modifiers(void *data, struct wl_keyboard *kb, uint32_t serial,
                      uint32_t depressed, uint32_t latched, uint32_t locked,
                      uint32_t group) {
    (void)data;
    (void)kb;
    (void)serial;
    (void)depressed;
    (void)latched;
    (void)locked;
    (void)group;
}
static void repeat(void *data, struct wl_keyboard *kb, int32_t rate,
                   int32_t delay) {
    (void)data;
    (void)kb;
    (void)rate;
    (void)delay;
}
static const struct wl_keyboard_listener keyboard_listener = {
    .keymap = keymap,
    .enter = enter,
    .leave = leave,
    .key = key,
    .modifiers = modifiers,
    .repeat_info = repeat,
};
static void capabilities(void *data, struct wl_seat *s, uint32_t caps) {
    (void)data;
    if ((caps & WL_SEAT_CAPABILITY_KEYBOARD) && !keyboard) {
        keyboard = wl_seat_get_keyboard(s);
        wl_keyboard_add_listener(keyboard, &keyboard_listener, NULL);
    }
}
static void seat_name(void *data, struct wl_seat *s, const char *name) {
    (void)data;
    (void)s;
    (void)name;
}
static const struct wl_seat_listener seat_listener = {
    .capabilities = capabilities,
    .name = seat_name,
};
static void decoration_configure(void *data,
                                 struct zxdg_toplevel_decoration_v1 *decoration,
                                 uint32_t mode) {
    (void)decoration;
    struct view *v = data;
    v->xdg_mode = mode;
    v->xdg_decoration_count++;
    printf("{\"event\":\"xdg-decoration\",\"name\":\"%s\",\"mode\":%u}\n",
           v->name, mode);
}
static const struct zxdg_toplevel_decoration_v1_listener decoration_listener = {
    .configure = decoration_configure};
static void kde_mode(void *data,
                     struct org_kde_kwin_server_decoration *decoration,
                     uint32_t mode) {
    (void)decoration;
    struct view *v = data;
    v->kde_mode = mode;
    v->kde_decoration_count++;
    printf("{\"event\":\"kde-decoration\",\"name\":\"%s\",\"mode\":%u}\n",
           v->name, mode);
}
static const struct org_kde_kwin_server_decoration_listener kde_listener = {
    .mode = kde_mode};
static void kde_default(void *data,
                        struct org_kde_kwin_server_decoration_manager *manager,
                        uint32_t mode) {
    (void)data;
    (void)manager;
    kde_default_mode = mode;
    kde_default_count++;
    printf("{\"event\":\"kde-default\",\"mode\":%u}\n", mode);
}
static const struct org_kde_kwin_server_decoration_manager_listener
    kde_manager_listener = {.default_mode = kde_default};
static void decorate(struct view *v, const char *protocol, const char *mode) {
    if (!v->top)
        fail("decoration requires XDG toplevel");
    if (!strcmp(protocol, "xdg")) {
        if (!xdg_decoration_manager || v->kde_decoration)
            fail("XDG decoration unavailable or mixed with KDE");
        if (!strcmp(mode, "destroy")) {
            if (!v->xdg_decoration)
                fail("no XDG decoration to destroy");
            zxdg_toplevel_decoration_v1_destroy(v->xdg_decoration);
            v->xdg_decoration = NULL;
            v->xdg_mode = 0;
            return;
        }
        if (!v->xdg_decoration) {
            // Bind v1 even when newer XML is installed: v1 forbids creating the
            // object on a buffered toplevel. Recreate between unmap and remap.
            if (v->committed_w)
                fail("XDG decoration creation requires unbuffered toplevel");
            v->xdg_mode = 0;
            v->xdg_decoration =
                zxdg_decoration_manager_v1_get_toplevel_decoration(
                    xdg_decoration_manager, v->top);
            zxdg_toplevel_decoration_v1_add_listener(v->xdg_decoration,
                                                     &decoration_listener, v);
        }
        if (!strcmp(mode, "unset"))
            zxdg_toplevel_decoration_v1_unset_mode(v->xdg_decoration);
        else if (!strcmp(mode, "server") || !strcmp(mode, "client"))
            zxdg_toplevel_decoration_v1_set_mode(
                v->xdg_decoration, !strcmp(mode, "server") ? 2 : 1);
        else if (strcmp(mode, "default"))
            fail("invalid XDG decoration mode");
    } else if (!strcmp(protocol, "kde")) {
        if (!kde_decoration_manager || v->xdg_decoration)
            fail("KDE decoration unavailable or mixed with XDG");
        if (!strcmp(mode, "destroy")) {
            if (!v->kde_decoration)
                fail("no KDE decoration to destroy");
            org_kde_kwin_server_decoration_release(v->kde_decoration);
            v->kde_decoration = NULL;
            v->kde_mode = 0;
            return;
        }
        if (!v->kde_decoration) {
            v->kde_mode = 0;
            v->kde_decoration = org_kde_kwin_server_decoration_manager_create(
                kde_decoration_manager, v->surface);
            org_kde_kwin_server_decoration_add_listener(v->kde_decoration,
                                                        &kde_listener, v);
        }
        if (!strcmp(mode, "server") || !strcmp(mode, "client") ||
            !strcmp(mode, "none"))
            org_kde_kwin_server_decoration_request_mode(
                v->kde_decoration, !strcmp(mode, "server")   ? 2
                                   : !strcmp(mode, "client") ? 1
                                                             : 0);
        else if (strcmp(mode, "default"))
            fail("invalid KDE decoration mode");
    } else
        fail("unknown decoration protocol");
}
static void output_geometry(void *data, struct wl_output *output, int32_t x, int32_t y,
                            int32_t w, int32_t h, int32_t subpixel, const char *make,
                            const char *model, int32_t transform) {
    (void)data;
    (void)output;
    (void)x;
    (void)y;
    (void)w;
    (void)h;
    (void)subpixel;
    (void)make;
    (void)model;
    (void)transform;
}
static void output_mode(void *data, struct wl_output *output, uint32_t flags,
                        int32_t w, int32_t h, int32_t refresh) {
    (void)data;
    (void)output;
    (void)flags;
    (void)w;
    (void)h;
    (void)refresh;
}
static const struct wl_output_listener output_listener = {
    .geometry = output_geometry, .mode = output_mode,
};
static void surface_enter(void *data, struct wl_surface *surface, struct wl_output *output) {
    (void)surface;
    (void)output;
    struct view *v = data;
    v->outputs++;
    if (v->animated && v->mapped && v->configured && !v->suspended)
        draw(v);
}
static void surface_leave(void *data, struct wl_surface *surface, struct wl_output *output) {
    (void)surface;
    (void)output;
    struct view *v = data;
    v->outputs--;
    if (v->outputs < 0)
        fail("unpaired output leave");
}
static const struct wl_surface_listener surface_listener = {
    .enter = surface_enter, .leave = surface_leave,
};
static void global(void *data, struct wl_registry *registry, uint32_t id,
                   const char *interface, uint32_t version) {
    (void)data;
    if (!strcmp(interface, "wl_compositor"))
        compositor = wl_registry_bind(registry, id, &wl_compositor_interface,
                                      4 < version ? 4 : version);
    else if (!strcmp(interface, "wl_subcompositor"))
        subcompositor = wl_registry_bind(registry, id,
                                         &wl_subcompositor_interface, 1);
    else if (!strcmp(interface, "wl_output")) {
        struct wl_output *output = wl_registry_bind(registry, id, &wl_output_interface, 1);
        wl_output_add_listener(output, &output_listener, NULL);
    } else if (!strcmp(interface, "wl_shm"))
        shm = wl_registry_bind(registry, id, &wl_shm_interface, 1);
    else if (!strcmp(interface, "xdg_wm_base")) {
        wm = wl_registry_bind(registry, id, &xdg_wm_base_interface, version < 6 ? version : 6);
        xdg_wm_base_add_listener(wm, &wm_listener, NULL);
    } else if (!strcmp(interface, "zwlr_layer_shell_v1"))
        shell = wl_registry_bind(registry, id, &zwlr_layer_shell_v1_interface,
                                 version < 2 ? version : 2);
    else if (!strcmp(interface, "zxdg_decoration_manager_v1"))
        xdg_decoration_manager = wl_registry_bind(
            registry, id, &zxdg_decoration_manager_v1_interface, 1);
    else if (!strcmp(interface, "org_kde_kwin_server_decoration_manager")) {
        kde_decoration_manager = wl_registry_bind(
            registry, id, &org_kde_kwin_server_decoration_manager_interface, 1);
        org_kde_kwin_server_decoration_manager_add_listener(
            kde_decoration_manager, &kde_manager_listener, NULL);
    } else if (!strcmp(interface, "wl_seat") && !seat) {
        seat = wl_registry_bind(registry, id, &wl_seat_interface,
                                version < 5 ? version : 5);
        wl_seat_add_listener(seat, &seat_listener, NULL);
    }
}
static void removed(void *data, struct wl_registry *registry, uint32_t id) {
    (void)data;
    (void)registry;
    (void)id;
}
static const struct wl_registry_listener registry_listener = {
    .global = global, .global_remove = removed};
static struct view *create(const char *name, uint32_t color) {
    if (count == 16)
        fail("too many views");
    struct view *v = &views[count++];
    snprintf(v->name, sizeof(v->name), "%s", name);
    v->color = color | 0xff000000;
    v->alpha = 255;
    v->surface = wl_compositor_create_surface(compositor);
    wl_surface_add_listener(v->surface, &surface_listener, v);
    return v;
}
static void destroy(struct view *v) {
    if (!v->surface)
        return;
    // Destroy descendants before their parent so no inert subsurface roles or
    // stale mapped children remain in fixture state.
    for (int i = 0; i < count; i++)
        if (views[i].parent == v)
            destroy(&views[i]);
    if (v->frame) {
        wl_callback_destroy(v->frame);
        v->frame = NULL;
    }
    v->animated = false;
    if (v->subsurface)
        wl_subsurface_destroy(v->subsurface);
    if (v->layer)
        zwlr_layer_surface_v1_destroy(v->layer);
    if (v->popup)
        xdg_popup_destroy(v->popup);
    if (v->xdg_decoration)
        zxdg_toplevel_decoration_v1_destroy(v->xdg_decoration);
    if (v->kde_decoration)
        org_kde_kwin_server_decoration_release(v->kde_decoration);
    v->xdg_decoration = NULL;
    v->kde_decoration = NULL;
    if (v->top)
        xdg_toplevel_destroy(v->top);
    if (v->xdg)
        xdg_surface_destroy(v->xdg);
    wl_surface_destroy(v->surface);
    v->subsurface = NULL;
    v->parent = NULL;
    v->layer = NULL;
    v->popup = NULL;
    v->top = NULL;
    v->xdg = NULL;
    v->surface = NULL;
    v->mapped = false;
    v->configured = false;
    v->committed_w = v->committed_h = 0;
}
static void command(char *line) {
    char op[32], name[32], parent[32], alpha_text[256];
    char protocol[16], mode[16], title[256] = "";
    int w, h, kind, zone, interactive, fixed, offset, anchors, x, y;
    unsigned color;
    if (sscanf(line, "%31s", op) != 1)
        return;
    if ((!strcmp(op, "app") || !strcmp(op, "app-maximized") ||
         !strcmp(op, "app-xdg") || !strcmp(op, "app-kde") ||
         !strcmp(op, "app-xdg-deferred")) &&
        sscanf(line, "%*s %31s %d %d %d %d %x", name, &w, &h, &fixed, &offset,
               &color) == 6) {
        struct view *v = create(name, color);
        v->requested_w = w;
        v->requested_h = h;
        v->fixed = fixed;
        v->offset = offset;
        v->mapped = strcmp(op, "app-xdg-deferred") != 0;
        v->xdg = xdg_wm_base_get_xdg_surface(wm, v->surface);
        xdg_surface_add_listener(v->xdg, &xdg_listener, v);
        v->top = xdg_surface_get_toplevel(v->xdg);
        xdg_toplevel_add_listener(v->top, &top_listener, v);
        xdg_toplevel_set_app_id(v->top, name);
        xdg_toplevel_set_title(v->top, name);
        if (!strcmp(op, "app-maximized"))
            xdg_toplevel_set_maximized(v->top);
        if (!strcmp(op, "app-xdg") || !strcmp(op, "app-kde") ||
            !strcmp(op, "app-xdg-deferred")) {
            if (sscanf(line, "%*s %*s %*d %*d %*d %*d %*x %15s", mode) != 1)
                fail("app decoration mode required");
            decorate(v, !strcmp(op, "app-kde") ? "kde" : "xdg", mode);
        }
        if (v->mapped)
            wl_surface_commit(v->surface);
    } else if (!strcmp(op, "decorate") &&
               sscanf(line, "%*s %31s %15s %15s", name, protocol, mode) == 3) {
        decorate(find(name), protocol, mode);
    } else if (!strcmp(op, "hold") && sscanf(line, "%*s %31s", name) == 1) {
        struct view *v = find(name);
        if (!v->top || !v->mapped || !v->configured)
            fail("hold requires mapped configured toplevel");
        v->hold_commit = true;
    } else if ((!strcmp(op, "remap") || !strcmp(op, "start")) &&
               sscanf(line, "%*s %31s", name) == 1) {
        struct view *v = find(name);
        if (!v->top || v->mapped || v->configured || v->committed_w)
            fail("start/remap requires unmapped unconfigured toplevel");
        // XDG unmap resets toplevel attributes. No old buffer may be reattached
        // until a fresh configure has been received and ACKed.
        xdg_toplevel_set_app_id(v->top, name);
        xdg_toplevel_set_title(v->top, name);
        v->mapped = true;
        wl_surface_commit(v->surface);
    } else if (!strcmp(op, "title") &&
               sscanf(line, "%*s %31s %255[^\n]", name, title) >= 1) {
        struct view *v = find(name);
        if (!v->top)
            fail("title requires XDG toplevel");
        xdg_toplevel_set_title(v->top, title);
    } else if (!strcmp(op, "pattern") && sscanf(line, "%*s %31s", name) == 1) {
        struct view *v = find(name);
        if (!v->mapped || !v->configured)
            fail("pattern requires mapped view");
        v->sharp_pattern = true;
        draw(v);
    } else if (!strcmp(op, "animate") && sscanf(line, "%*s %31s", name) == 1) {
        struct view *v = find(name);
        if (!v->mapped || !v->configured)
            fail("animate requires mapped view");
        v->animated = true;
        draw(v);
    } else if (!strcmp(op, "alpha") &&
               sscanf(line, "%*s %31s %255s", name, alpha_text) == 2) {
        struct view *v = find(name);
        char *end;
        errno = 0;
        long alpha = strtol(alpha_text, &end, 10);
        if (errno || end == alpha_text || *end || alpha < 0 || alpha > 255)
            fail("alpha must be 0..255");
        if (!v->surface || !v->mapped || !v->configured)
            fail("alpha requires mapped configured view");
        v->alpha = (uint32_t)alpha;
        v->hold_commit = false;
        draw(v);
    } else if (!strcmp(op, "subsurface") &&
               sscanf(line, "%*s %31s %31s %d %d %d %d %x", name, parent, &w,
                      &h, &x, &y, &color) == 7) {
        struct view *p = find(parent);
        if (!subcompositor || !p->surface)
            fail("subsurface requires wl_subcompositor and live parent");
        if (w <= 0 || h <= 0 || w > 4096 || h > 4096)
            fail("invalid subsurface size");
        for (int i = 0; i < count; i++)
            if (!strcmp(views[i].name, name))
                fail("subsurface name already used");
        struct view *v = create(name, color);
        v->parent = p;
        v->width = v->requested_w = w;
        v->height = v->requested_h = h;
        v->fixed = true;
        // Subsurfaces have no configure handshake or XDG role.
        v->mapped = v->configured = true;
        v->subsurface = wl_subcompositor_get_subsurface(
            subcompositor, v->surface, p->surface);
        wl_subsurface_set_position(v->subsurface, x, y);
        wl_subsurface_set_desync(v->subsurface);
        draw(v);
        // Position is parent-commit state even for a desynchronized child.
        // All fixture subsurface ancestors are also desynchronized.
        wl_surface_commit(p->surface);
    } else if ((!strcmp(op, "maximize") || !strcmp(op, "unmaximize") ||
                !strcmp(op, "minimize")) &&
               sscanf(line, "%*s %31s", name) == 1) {
        struct view *v = find(name);
        if (!v->top)
            fail("window state requires XDG toplevel");
        if (!strcmp(op, "maximize"))
            xdg_toplevel_set_maximized(v->top);
        else if (!strcmp(op, "unmaximize"))
            xdg_toplevel_unset_maximized(v->top);
        else
            xdg_toplevel_set_minimized(v->top);
    } else if (!strcmp(op, "layer") &&
               sscanf(line, "%*s %31s %d %d %d %d %d %x", name, &kind, &w, &h,
                      &zone, &interactive, &color) == 7) {
        struct view *v = create(name, color);
        v->layer = zwlr_layer_shell_v1_get_layer_surface(
            shell, v->surface, NULL, (uint32_t)kind, name);
        zwlr_layer_surface_v1_add_listener(v->layer, &layer_listener, v);
        zwlr_layer_surface_v1_set_size(v->layer, (uint32_t)w, (uint32_t)h);
        // Width zero makes a full-width top panel; fixed sizes are centered
        // overlays.
        zwlr_layer_surface_v1_set_anchor(v->layer, w == 0 ? 1 | 4 | 8 : 0);
        zwlr_layer_surface_v1_set_exclusive_zone(v->layer, zone);
        zwlr_layer_surface_v1_set_keyboard_interactivity(v->layer,
                                                         (uint32_t)interactive);
        wl_surface_commit(v->surface);
    } else if (!strcmp(op, "popup") &&
               sscanf(line, "%*s %31s %31s %d %d %d %d %x", name, parent, &x,
                      &y, &w, &h, &color) == 7) {
        struct view *p = find(parent);
        if ((!p->layer && !p->top) || !p->mapped)
            fail("popup requires mapped layer or toplevel parent");
        struct view *v = create(name, color);
        v->mapped = true;
        v->parent = p;
        v->xdg = xdg_wm_base_get_xdg_surface(wm, v->surface);
        xdg_surface_add_listener(v->xdg, &xdg_listener, v);
        struct xdg_positioner *positioner = xdg_wm_base_create_positioner(wm);
        xdg_positioner_set_size(positioner, w, h);
        xdg_positioner_set_anchor_rect(positioner, x, y - 1, 1, 1);
        xdg_positioner_set_anchor(positioner,
                                  XDG_POSITIONER_ANCHOR_BOTTOM_LEFT);
        xdg_positioner_set_gravity(positioner,
                                   XDG_POSITIONER_GRAVITY_BOTTOM_RIGHT);
        // Layer-shell supplies the parent before the popup's initial commit.
        v->popup =
            xdg_surface_get_popup(v->xdg, p->top ? p->xdg : NULL, positioner);
        xdg_popup_add_listener(v->popup, &popup_listener, v);
        if (p->layer)
            zwlr_layer_surface_v1_get_popup(p->layer, v->popup);
        xdg_positioner_destroy(positioner);
        wl_surface_commit(v->surface);
    } else if (!strcmp(op, "reconfigure") &&
               sscanf(line, "%*s %31s %d %d %d %d %d", name, &w, &h, &anchors,
                      &zone, &interactive) == 6) {
        struct view *v = find(name);
        if (!v->layer || v->mapped || v->configured)
            fail("reconfigure requires unmapped, unconfigured layer");
        zwlr_layer_surface_v1_set_size(v->layer, (uint32_t)w, (uint32_t)h);
        zwlr_layer_surface_v1_set_anchor(v->layer, (uint32_t)anchors);
        zwlr_layer_surface_v1_set_exclusive_zone(v->layer, zone);
        zwlr_layer_surface_v1_set_keyboard_interactivity(v->layer,
                                                         (uint32_t)interactive);
        // A fresh bufferless handshake is mandatory after a null-buffer unmap.
        wl_surface_commit(v->surface);
    } else if (!strcmp(op, "set-layer") &&
               sscanf(line, "%*s %31s %d", name, &kind) == 2) {
        struct view *v = find(name);
        if (!v->layer || zwlr_layer_surface_v1_get_version(v->layer) < 2)
            fail("set_layer requires layer-shell v2");
        // Do not accidentally apply pending state via a configure-triggered
        // draw.
        v->hold_commit = true;
        zwlr_layer_surface_v1_set_layer(v->layer, (uint32_t)kind);
    } else if (!strcmp(op, "commit") && sscanf(line, "%*s %31s", name) == 1) {
        struct view *v = find(name);
        if (!v->mapped || !v->configured)
            fail("commit requires mapped configured view");
        v->hold_commit = false;
        draw(v);
    } else if (!strcmp(op, "sync")) {
        if (wl_display_roundtrip(display) < 0)
            fail("sync");
    } else if (!strcmp(op, "map") && sscanf(line, "%*s %31s", name) == 1) {
        struct view *v = find(name);
        if (!v->configured)
            fail("map before configure");
        v->mapped = true;
        draw(v);
    } else if (!strcmp(op, "resize") &&
               sscanf(line, "%*s %31s %d %d %d", name, &w, &h, &zone) == 4) {
        struct view *v = find(name);
        if (v->layer) {
            zwlr_layer_surface_v1_set_size(v->layer, (uint32_t)w, (uint32_t)h);
            zwlr_layer_surface_v1_set_exclusive_zone(v->layer, zone);
            wl_surface_commit(v->surface);
        } else {
            v->requested_w = w;
            v->requested_h = h;
            draw(v);
        }
    } else if (!strcmp(op, "unmap") && sscanf(line, "%*s %31s", name) == 1) {
        struct view *v = find(name);
        v->mapped = false;
        wl_surface_attach(v->surface, NULL, 0, 0);
        wl_surface_commit(v->surface);
        v->committed_w = v->committed_h = 0;
        v->configured = v->subsurface != NULL;
        v->hold_commit = false;
    } else if (!strcmp(op, "destroy") && sscanf(line, "%*s %31s", name) == 1) {
        destroy(find(name));
    } else if (!strcmp(op, "state")) {
        printf("{\"event\":\"state\",\"focus\":\"%s\",\"kde_default_mode\":%u,"
               "\"kde_default_count\":%u,\"views\":{",
               surface_name(focus), kde_default_mode, kde_default_count);
        for (int i = 0; i < count; i++) {
            struct view *v = &views[i];
            printf(
                "%s\"%s\":{\"width\":%d,\"height\":%d,"
                "\"configure_width\":%d,\"configure_height\":%d,"
                "\"configure_count\":%u,\"commit_count\":%u,"
                "\"frame_count\":%u,\"color\":%u,\"outputs\":%d,\"suspended\":%s,"
                "\"popup_x\":%d,\"popup_y\":%d,\"configured\":%s,\"mapped\":"
                "%s,\"maximized\":%s,\"hold_commit\":%s,\"ack_serial\":%u,"
                "\"xdg_decoration\":%s,\"kde_decoration\":%s,"
                "\"xdg_mode\":%u,\"kde_mode\":%u,"
                "\"xdg_decoration_count\":%u,\"kde_decoration_count\":%u}",
                i ? "," : "", v->name, v->committed_w, v->committed_h,
                v->configure_w, v->configure_h, v->configure_count,
                v->commit_count, v->frame_count, v->color, v->outputs,
                v->suspended ? "true" : "false", v->popup_x, v->popup_y,
                v->configured ? "true" : "false", v->mapped ? "true" : "false",
                v->maximized ? "true" : "false",
                v->hold_commit ? "true" : "false", v->ack_serial,
                v->xdg_decoration ? "true" : "false",
                v->kde_decoration ? "true" : "false", v->xdg_mode, v->kde_mode,
                v->xdg_decoration_count, v->kde_decoration_count);
        }
        puts("}}");
    } else
        fail("invalid command");
    puts("{\"event\":\"done\"}");
}
int main(void) {
    alarm(45);
    setvbuf(stdout, NULL, _IOLBF, 0);
    setvbuf(stdin, NULL, _IONBF, 0);
    display = wl_display_connect(NULL);
    if (!display)
        fail("connect");
    struct wl_registry *registry = wl_display_get_registry(display);
    wl_registry_add_listener(registry, &registry_listener, NULL);
    if (wl_display_roundtrip(display) < 0 || wl_display_roundtrip(display) < 0)
        fail("registry");
    if (!compositor || !shm || !wm || !shell || !keyboard)
        fail("required Wayland globals missing");
    puts("{\"event\":\"ready\"}");
    for (;;) {
        if (wl_display_dispatch_pending(display) < 0 ||
            wl_display_flush(display) < 0)
            fail("dispatch/flush");
        struct pollfd fds[] = {{wl_display_get_fd(display), POLLIN, 0},
                               {STDIN_FILENO, POLLIN, 0}};
        if (poll(fds, 2, 2000) < 0)
            fail("poll");
        if (fds[0].revents & (POLLERR | POLLHUP))
            fail("display disconnected");
        if ((fds[0].revents & POLLIN) && wl_display_dispatch(display) < 0)
            fail("dispatch");
        if (fds[1].revents & (POLLIN | POLLHUP)) {
            char line[256];
            if (!fgets(line, sizeof(line), stdin))
                break;
            command(line);
        }
    }
    wl_display_disconnect(display);
    return 0;
}
