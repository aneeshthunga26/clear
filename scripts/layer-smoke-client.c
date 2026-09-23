// Real SHM-backed Wayland clients for vm-layer-smoke.py; no compositor test
// hooks.
#define _GNU_SOURCE
#include "wlr-layer-shell-client-protocol.h"
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
    struct xdg_surface *xdg;
    struct xdg_toplevel *top;
    struct xdg_popup *popup;
    struct zwlr_layer_surface_v1 *layer;
    int width, height, requested_w, requested_h, offset;
    int committed_w, committed_h, configure_w, configure_h, popup_x, popup_y;
    unsigned configure_count, commit_count;
    bool hold_commit;
    uint32_t color;
    bool mapped, configured, fixed;
};
static struct wl_display *display;
static struct wl_compositor *compositor;
static struct wl_shm *shm;
static struct xdg_wm_base *wm;
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
    // A contrasting buffer margin detects placement using surface rather than
    // XDG geometry.
    for (int y = 0; y < bh; y++)
        for (int x = 0; x < bw; x++)
            p[y * bw + x] = x >= v->offset && x < w + v->offset &&
                                    y >= v->offset && y < h + v->offset
                                ? v->color
                                : 0xffa030c0;
    struct wl_shm_pool *pool = wl_shm_create_pool(shm, fd, (int)size);
    struct wl_buffer *buffer = wl_shm_pool_create_buffer(
        pool, 0, bw, bh, bw * 4, WL_SHM_FORMAT_XRGB8888);
    wl_buffer_add_listener(buffer, &buffer_listener, pixels);
    wl_shm_pool_destroy(pool);
    close(fd);
    if (v->xdg)
        xdg_surface_set_window_geometry(v->xdg, v->offset, v->offset, w, h);
    wl_surface_attach(v->surface, buffer, 0, 0);
    wl_surface_damage(v->surface, 0, 0, bw, bh);
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
    v->configured = true;
    if (v->mapped)
        draw(v);
}
static const struct xdg_surface_listener xdg_listener = {.configure =
                                                             xdg_configure};
static void top_configure(void *data, struct xdg_toplevel *top, int32_t w,
                          int32_t h, struct wl_array *states) {
    (void)top;
    (void)states;
    struct view *v = data;
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
static const struct xdg_toplevel_listener top_listener = {
    .configure = top_configure,
    .close = top_close,
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
static void global(void *data, struct wl_registry *registry, uint32_t id,
                   const char *interface, uint32_t version) {
    (void)data;
    if (!strcmp(interface, "wl_compositor"))
        compositor = wl_registry_bind(registry, id, &wl_compositor_interface,
                                      4 < version ? 4 : version);
    else if (!strcmp(interface, "wl_shm"))
        shm = wl_registry_bind(registry, id, &wl_shm_interface, 1);
    else if (!strcmp(interface, "xdg_wm_base")) {
        wm = wl_registry_bind(registry, id, &xdg_wm_base_interface, 1);
        xdg_wm_base_add_listener(wm, &wm_listener, NULL);
    } else if (!strcmp(interface, "zwlr_layer_shell_v1"))
        shell = wl_registry_bind(registry, id, &zwlr_layer_shell_v1_interface,
                                 version < 2 ? version : 2);
    else if (!strcmp(interface, "wl_seat") && !seat) {
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
    v->surface = wl_compositor_create_surface(compositor);
    return v;
}
static void command(char *line) {
    char op[32], name[32], parent[32];
    int w, h, kind, zone, interactive, fixed, offset, anchors, x, y;
    unsigned color;
    if (sscanf(line, "%31s", op) != 1)
        return;
    if (!strcmp(op, "app") && sscanf(line, "%*s %31s %d %d %d %d %x", name, &w,
                                     &h, &fixed, &offset, &color) == 6) {
        struct view *v = create(name, color);
        v->requested_w = w;
        v->requested_h = h;
        v->fixed = fixed;
        v->offset = offset;
        v->mapped = true;
        v->xdg = xdg_wm_base_get_xdg_surface(wm, v->surface);
        xdg_surface_add_listener(v->xdg, &xdg_listener, v);
        v->top = xdg_surface_get_toplevel(v->xdg);
        xdg_toplevel_add_listener(v->top, &top_listener, v);
        xdg_toplevel_set_app_id(v->top, name);
        xdg_toplevel_set_title(v->top, name);
        wl_surface_commit(v->surface);
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
        if (!p->layer || !p->mapped)
            fail("popup requires mapped layer parent");
        struct view *v = create(name, color);
        v->mapped = true;
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
        v->popup = xdg_surface_get_popup(v->xdg, NULL, positioner);
        xdg_popup_add_listener(v->popup, &popup_listener, v);
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
        v->configured = false;
        v->hold_commit = false;
    } else if (!strcmp(op, "destroy") && sscanf(line, "%*s %31s", name) == 1) {
        struct view *v = find(name);
        if (v->layer)
            zwlr_layer_surface_v1_destroy(v->layer);
        if (v->popup)
            xdg_popup_destroy(v->popup);
        if (v->top)
            xdg_toplevel_destroy(v->top);
        if (v->xdg)
            xdg_surface_destroy(v->xdg);
        wl_surface_destroy(v->surface);
        v->surface = NULL;
        v->mapped = false;
        v->configured = false;
        v->committed_w = v->committed_h = 0;
    } else if (!strcmp(op, "state")) {
        printf("{\"event\":\"state\",\"focus\":\"%s\",\"views\":{",
               surface_name(focus));
        for (int i = 0; i < count; i++) {
            struct view *v = &views[i];
            printf("%s\"%s\":{\"width\":%d,\"height\":%d,"
                   "\"configure_width\":%d,\"configure_height\":%d,"
                   "\"configure_count\":%u,\"commit_count\":%u,"
                   "\"popup_x\":%d,\"popup_y\":%d,\"configured\":%s,\"mapped\":"
                   "%s}",
                   i ? "," : "", v->name, v->committed_w, v->committed_h,
                   v->configure_w, v->configure_h, v->configure_count,
                   v->commit_count, v->popup_x, v->popup_y,
                   v->configured ? "true" : "false",
                   v->mapped ? "true" : "false");
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
