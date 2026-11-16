#define _GNU_SOURCE
#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <time.h>
#include <unistd.h>
#include <wayland-client.h>
#include <xkbcommon/xkbcommon.h>
#include "virtual-keyboard-client.h"
#include "virtual-pointer-client.h"

static struct wl_display *display;
static struct wl_seat *seat;
static struct zwp_virtual_keyboard_manager_v1 *manager;
static struct zwlr_virtual_pointer_manager_v1 *pmanager;
static struct zwp_virtual_keyboard_v1 *keyboard;
static struct zwlr_virtual_pointer_v1 *pointer;
static struct xkb_context *context;
static struct xkb_keymap *keymap;
static struct xkb_state *state;

static void global(void *data, struct wl_registry *registry, uint32_t name,
                   const char *interface, uint32_t version) {
    if (!strcmp(interface, "wl_seat"))
        seat = wl_registry_bind(registry, name, &wl_seat_interface, 1);
    if (!strcmp(interface, "zwp_virtual_keyboard_manager_v1"))
        manager = wl_registry_bind(registry, name, &zwp_virtual_keyboard_manager_v1_interface, 1);
    if (!strcmp(interface, "zwlr_virtual_pointer_manager_v1"))
        pmanager = wl_registry_bind(registry, name, &zwlr_virtual_pointer_manager_v1_interface, 1);
}
static void removed(void *data, struct wl_registry *registry, uint32_t name) {}
static const struct wl_registry_listener listener = {global, removed};

static uint32_t millis(void) {
    struct timespec now;
    clock_gettime(CLOCK_MONOTONIC, &now);
    return now.tv_sec * 1000 + now.tv_nsec / 1000000;
}
static void modifiers(void) {
    zwp_virtual_keyboard_v1_modifiers(keyboard,
        xkb_state_serialize_mods(state, XKB_STATE_MODS_DEPRESSED),
        xkb_state_serialize_mods(state, XKB_STATE_MODS_LATCHED),
        xkb_state_serialize_mods(state, XKB_STATE_MODS_LOCKED),
        xkb_state_serialize_layout(state, XKB_STATE_LAYOUT_EFFECTIVE));
}
static void layout(const char *name, const char *variant) {
    struct xkb_rule_names names = {.rules="evdev", .model="pc105", .layout=name, .variant=variant};
    if (state) xkb_state_unref(state);
    if (keymap) xkb_keymap_unref(keymap);
    keymap = xkb_keymap_new_from_names(context, &names, XKB_KEYMAP_COMPILE_NO_FLAGS);
    assert(keymap);
    state = xkb_state_new(keymap);
    assert(state);
    if (!strcmp(name, "us") && !*variant)
        assert(xkb_state_key_get_one_sym(state, 30 + 8) == 'a');
    char *text = xkb_keymap_get_as_string(keymap, XKB_KEYMAP_FORMAT_TEXT_V1);
    size_t size = strlen(text) + 1;
    int fd = memfd_create("rune-spike-fixed-keymap", MFD_CLOEXEC);
    assert(fd >= 0 && write(fd, text, size) == size);
    zwp_virtual_keyboard_v1_keymap(keyboard, 1, fd, size);
    close(fd);
    free(text);
    modifiers();
    wl_display_roundtrip(display);
    printf("layout %s %s: evdev pc105\n", name, variant);
}
static void key(unsigned code, int pressed) {
    zwp_virtual_keyboard_v1_key(keyboard, millis(), code, pressed);
    xkb_state_update_key(state, code + 8, pressed ? XKB_KEY_DOWN : XKB_KEY_UP);
    modifiers();
    assert(wl_display_roundtrip(display) >= 0);
    printf("key evdev=%u %s\n", code, pressed ? "down" : "up");
}
int main(int argc, char **argv) {
    const char *runtime = getenv("XDG_RUNTIME_DIR");
    const char *state = getenv("RUNE_SYNC_STATE");
    assert(state);
    char expected[4096];
    int length = snprintf(expected, sizeof expected, "%s/xdg", state);
    assert(length > 0 && (size_t)length < sizeof expected);
    assert(runtime && !strcmp(runtime, expected));
    assert(!strcmp(getenv("WAYLAND_DISPLAY"), "wayland-1"));
    setbuf(stdout, NULL);
    display = wl_display_connect(NULL);
    assert(display);
    struct wl_registry *registry = wl_display_get_registry(display);
    wl_registry_add_listener(registry, &listener, NULL);
    assert(wl_display_roundtrip(display) >= 0);
    assert(manager && seat && pmanager);
    keyboard = zwp_virtual_keyboard_manager_v1_create_virtual_keyboard(manager, seat);
    pointer = zwlr_virtual_pointer_manager_v1_create_virtual_pointer(pmanager, seat);
    context = xkb_context_new(XKB_CONTEXT_NO_FLAGS);
    layout("us", "");
    FILE *input = argc == 2 ? fopen(argv[1], "r+") : stdin;
    assert(input);
    char line[256], op[32], a[64], b[64];
    while (fgets(line, sizeof(line), input)) {
        a[0] = b[0] = 0;
        if (sscanf(line, "%31s %63s %63s", op, a, b) < 1) continue;
        unsigned code = strtoul(a, NULL, 10);
        if (!strcmp(op, "quit")) break;
        else if (!strcmp(op, "sync")) printf("sync %s\n", a);
        else if (!strcmp(op, "map")) layout(a, b);
        else if (!strcmp(op, "down")) key(code, 1);
        else if (!strcmp(op, "up")) key(code, 0);
        else if (!strcmp(op, "tap")) { key(code, 1); usleep(40000); key(code, 0); }
        else if (!strcmp(op, "sleep")) usleep(code * 1000);
        else if (!strcmp(op, "move")) {
            zwlr_virtual_pointer_v1_motion_absolute(pointer, millis(), code, strtoul(b, NULL, 10), 1920, 1080);
            zwlr_virtual_pointer_v1_frame(pointer);
        } else if (!strcmp(op, "button")) {
            zwlr_virtual_pointer_v1_button(pointer, millis(), code, !strcmp(b, "down"));
            zwlr_virtual_pointer_v1_frame(pointer);
        } else { fprintf(stderr, "unknown command: %s", line); abort(); }
        assert(wl_display_roundtrip(display) >= 0);
    }
    zwp_virtual_keyboard_v1_destroy(keyboard);
    zwlr_virtual_pointer_v1_destroy(pointer);
    wl_display_roundtrip(display);
    wl_display_disconnect(display);
}
