/*
 * Hostable's JavaScript rules host: QuickJS (quickjs-ng) compiled to a wasm
 * module that speaks the rules ABI (see crates/rules-api/src/abi.rs).
 *
 * The game's script is loaded once with hostable_load(). It must define
 * globalThis.__hostable_dispatch(requestJson) -> responseJson; the
 * @hostable/sdk package does that for you.
 *
 * Licensed MIT OR Apache-2.0, like the rest of Hostable. QuickJS itself is
 * MIT licensed (see quickjs-ng's LICENSE).
 */
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <stdint.h>
#include "quickjs.h"

__attribute__((import_module("env"), import_name("log")))
extern void host_log(const char *ptr, uint32_t len);

static JSRuntime *rt; static JSContext *ctx;

__attribute__((export_name("hostable_alloc"))) void *hostable_alloc(uint32_t n) { return malloc(n ? n : 1); }
__attribute__((export_name("hostable_free"))) void hostable_free(void *p, uint32_t n) { (void)n; free(p); }

static char *out_buf;
static uint64_t pack(const char *s, size_t n) {
  free(out_buf); out_buf = malloc(n + 1); memcpy(out_buf, s, n);
  return ((uint64_t)(uintptr_t)out_buf << 32) | (uint64_t)n;
}
/* {"err":{"kind":"crashed","message":"..."}} with the message JSON-escaped
 * and cut to 400 characters. */
static uint64_t err(const char *m) {
  char b[2048]; size_t n = 0;
  const char *head = "{\"err\":{\"kind\":\"crashed\",\"message\":\"";
  memcpy(b, head, strlen(head)); n = strlen(head);
  for (size_t i = 0; m[i] && i < 400; i++) {
    unsigned char c = (unsigned char)m[i];
    if (c == '"' || c == '\\') { b[n++] = '\\'; b[n++] = (char)c; }
    else if (c < 0x20) { n += (size_t)snprintf(b + n, 8, "\\u%04x", c); }
    else b[n++] = (char)c;
  }
  memcpy(b + n, "\"}}", 3); n += 3;
  return pack(b, n);
}
static JSValue js_log(JSContext *c, JSValueConst this_val, int argc, JSValueConst *argv) {
  size_t n; const char *s = JS_ToCStringLen(c, &n, argv[0]); if (s) { host_log(s, n); JS_FreeCString(c, s);} return JS_UNDEFINED;
}
__attribute__((export_name("hostable_load"))) uint64_t hostable_load(const char *src, uint32_t len) {
  if (ctx) return err("already loaded");
  rt = JS_NewRuntime(); JS_SetMemoryLimit(rt, 48u<<20); JS_SetMaxStackSize(rt, 512*1024);
  ctx = JS_NewContext(rt);
  JSValue g = JS_GetGlobalObject(ctx);
  JS_SetPropertyStr(ctx, g, "__hostable_log", JS_NewCFunction(ctx, js_log, "log", 1));
  JS_FreeValue(ctx, g);
  JSValue r = JS_Eval(ctx, src, len, "rules.js", JS_EVAL_TYPE_GLOBAL);
  if (JS_IsException(r)) { JSValue e = JS_GetException(ctx); const char *m = JS_ToCString(ctx, e); uint64_t o = err(m ? m : "load failed"); JS_FreeCString(ctx, m); JS_FreeValue(ctx,e); return o; }
  JS_FreeValue(ctx, r); return pack("{\"ok\":null}", 11);
}
__attribute__((export_name("hostable_abi_version"))) int32_t hostable_abi_version(void) { return 1; }

__attribute__((export_name("hostable_call"))) uint64_t hostable_call(const char *req, uint32_t len) {
  if (!ctx) return err("no script loaded");
  JSValue g = JS_GetGlobalObject(ctx);
  JSValue f = JS_GetPropertyStr(ctx, g, "__hostable_dispatch");
  JSValue arg = JS_NewStringLen(ctx, req, len);
  JSValue r = JS_Call(ctx, f, g, 1, &arg);
  JS_FreeValue(ctx, arg); JS_FreeValue(ctx, f); JS_FreeValue(ctx, g);
  if (JS_IsException(r)) { JSValue e = JS_GetException(ctx); const char *m = JS_ToCString(ctx, e); uint64_t o = err(m ? m : "call failed"); JS_FreeCString(ctx, m); JS_FreeValue(ctx, e); return o; }
  if (!JS_IsString(r)) { JS_FreeValue(ctx, r); return err("__hostable_dispatch must return a string"); }
  size_t n; const char *s = JS_ToCStringLen(ctx, &n, r);
  if (!s) { JS_FreeValue(ctx, r); return err("out of memory"); }
  uint64_t o = pack(s, n); JS_FreeCString(ctx, s); JS_FreeValue(ctx, r); return o;
}

/* WASI stubs: the rules host gets no clock, files or output. Defining these
 * here keeps wasi-libc from importing them, so the module's only import is
 * env.log. The clock is frozen at 0 so rules stay deterministic. */
typedef struct { uint32_t buf; uint32_t len; } ciovec;
int32_t __imported_wasi_snapshot_preview1_clock_time_get(int32_t id, int64_t prec, int32_t out) {
  (void)id; (void)prec; *(uint64_t *)(uintptr_t)out = 0; return 0; }
int32_t __imported_wasi_snapshot_preview1_fd_close(int32_t fd) { (void)fd; return 8; }
int32_t __imported_wasi_snapshot_preview1_fd_fdstat_get(int32_t fd, int32_t out) { (void)fd; (void)out; return 8; }
int32_t __imported_wasi_snapshot_preview1_fd_seek(int32_t fd, int64_t off, int32_t wh, int32_t out) { (void)fd; (void)off; (void)wh; (void)out; return 8; }
int32_t __imported_wasi_snapshot_preview1_fd_write(int32_t fd, int32_t iovs, int32_t n, int32_t out) {
  (void)fd; uint32_t total = 0; const ciovec *v = (const ciovec *)(uintptr_t)iovs;
  for (int32_t i = 0; i < n; i++) { host_log((const char *)(uintptr_t)v[i].buf, v[i].len); total += v[i].len; }
  *(uint32_t *)(uintptr_t)out = total; return 0; }
