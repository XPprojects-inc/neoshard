//! Browser driver (wasm-bindgen). JS sees exactly four calls (ADR-0009):
//! `dispatch(commandBytes)`, `drain() -> updateBatchBytes`, `restore(snapshot)`
//! and `onPersist(cb)`. WebSocket, fetch and timers are driven from Rust.

use std::cell::RefCell;
use std::rc::Rc;

use neoshard_proto::core_api::Command;
use prost::Message as _;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{BinaryType, MessageEvent, Request, RequestInit, Response, WebSocket};

use crate::core::{Core, Effect, HttpRequest, HttpResponse};

fn now() -> u64 {
    js_sys::Date::now() as u64
}

struct Socket {
    ws: WebSocket,
    _on_open: Closure<dyn FnMut()>,
    _on_message: Closure<dyn FnMut(MessageEvent)>,
    _on_close: Closure<dyn FnMut()>,
}

struct Inner {
    core: Core,
    socket: Option<Socket>,
    generation: u32,
    persist: Option<js_sys::Function>,
    notify: Option<js_sys::Function>,
}

type Shared = Rc<RefCell<Inner>>;

#[wasm_bindgen]
pub struct WasmCore {
    inner: Shared,
    _tick: Closure<dyn FnMut()>,
}

fn detach(s: Socket) {
    s.ws.set_onopen(None);
    s.ws.set_onmessage(None);
    s.ws.set_onclose(None);
    let _ = s.ws.close_with_code(4000);
}

fn run_effects(inner: &Shared) {
    loop {
        let effects = inner.borrow_mut().core.take_effects();
        if effects.is_empty() {
            break;
        }
        for e in effects {
            match e {
                Effect::WsConnect(url) => connect(inner, &url),
                Effect::WsSend(b) => {
                    if let Some(s) = &inner.borrow().socket {
                        let _ = s.ws.send_with_u8_array(&b);
                    }
                }
                Effect::WsClose => {
                    let s = inner.borrow_mut().socket.take();
                    if let Some(s) = s {
                        detach(s);
                    }
                }
                Effect::Http(req) => http(inner.clone(), req),
                Effect::Persist(bytes) => {
                    let f = inner.borrow().persist.clone();
                    if let Some(f) = f {
                        let _ =
                            f.call1(&JsValue::NULL, &js_sys::Uint8Array::from(bytes.as_slice()));
                    }
                }
            }
        }
    }
    let (has, notify) = {
        let i = inner.borrow();
        (i.core.has_updates(), i.notify.clone())
    };
    if has && let Some(n) = notify {
        let _ = n.call0(&JsValue::NULL);
    }
}

fn connect(inner: &Shared, url: &str) {
    let old = inner.borrow_mut().socket.take();
    if let Some(s) = old {
        detach(s);
    }
    let generation = {
        let mut i = inner.borrow_mut();
        i.generation = i.generation.wrapping_add(1);
        i.generation
    };
    let Ok(ws) = WebSocket::new(url) else {
        inner.borrow_mut().core.on_ws_close(now());
        return;
    };
    ws.set_binary_type(BinaryType::Arraybuffer);
    let current = move |inner: &Shared| inner.borrow().generation == generation;

    let i1 = inner.clone();
    let on_open = Closure::<dyn FnMut()>::new(move || {
        if current(&i1) {
            i1.borrow_mut().core.on_ws_open(now());
            run_effects(&i1);
        }
    });
    let i2 = inner.clone();
    let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
        if !current(&i2) {
            return;
        }
        if let Ok(buf) = e.data().dyn_into::<js_sys::ArrayBuffer>() {
            let bytes = js_sys::Uint8Array::new(&buf).to_vec();
            i2.borrow_mut().core.on_ws_frame(&bytes, now());
            run_effects(&i2);
        }
    });
    let i3 = inner.clone();
    let on_close = Closure::<dyn FnMut()>::new(move || {
        if current(&i3) {
            i3.borrow_mut().socket = None;
            i3.borrow_mut().core.on_ws_close(now());
            run_effects(&i3);
        }
    });
    ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));
    ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));
    inner.borrow_mut().socket = Some(Socket {
        ws,
        _on_open: on_open,
        _on_message: on_message,
        _on_close: on_close,
    });
}

async fn fetch(req: &HttpRequest) -> Result<HttpResponse, JsValue> {
    let opts = RequestInit::new();
    opts.set_method(req.method);
    if let Some(b) = &req.body {
        opts.set_body(&JsValue::from_str(b));
    }
    let r = Request::new_with_str_and_init(&req.url, &opts)?;
    if let Some(a) = &req.auth {
        r.headers().set("authorization", a)?;
    }
    if req.body.is_some() {
        r.headers().set("content-type", "application/json")?;
    }
    let global: web_sys::Window = js_sys::global().unchecked_into();
    let resp: Response = JsFuture::from(global.fetch_with_request(&r))
        .await?
        .dyn_into()?;
    let retry_after_ms = resp
        .headers()
        .get("retry-after")
        .ok()
        .flatten()
        .and_then(|v| v.parse::<f64>().ok())
        .map(|s| (s * 1000.0) as u64)
        .unwrap_or(0);
    let body = JsFuture::from(resp.text()?)
        .await?
        .as_string()
        .unwrap_or_default();
    Ok(HttpResponse {
        id: req.id,
        status: resp.status(),
        body,
        retry_after_ms,
    })
}

fn http(inner: Shared, req: HttpRequest) {
    spawn_local(async move {
        let resp = fetch(&req).await.unwrap_or(HttpResponse {
            id: req.id,
            status: 0,
            body: String::new(),
            retry_after_ms: 0,
        });
        inner.borrow_mut().core.on_http(resp, now());
        run_effects(&inner);
    });
}

#[wasm_bindgen]
impl WasmCore {
    #[wasm_bindgen(constructor)]
    pub fn new(client_name: &str) -> WasmCore {
        let inner: Shared = Rc::new(RefCell::new(Inner {
            core: Core::new(client_name, now()),
            socket: None,
            generation: 0,
            persist: None,
            notify: None,
        }));
        let it = inner.clone();
        let tick = Closure::<dyn FnMut()>::new(move || {
            it.borrow_mut().core.tick(now());
            run_effects(&it);
        });
        let global: web_sys::Window = js_sys::global().unchecked_into();
        let _ = global.set_interval_with_callback_and_timeout_and_arguments_0(
            tick.as_ref().unchecked_ref(),
            100,
        );
        WasmCore { inner, _tick: tick }
    }

    /// Restore an offline snapshot (from IndexedDB). Returns false if it is unreadable.
    pub fn restore(&self, snapshot: &[u8]) -> bool {
        let ok = self.inner.borrow_mut().core.restore(snapshot);
        run_effects(&self.inner);
        ok
    }

    /// Send a protobuf-encoded `neoshard.core.v1.Command`.
    pub fn dispatch(&self, command: &[u8]) -> Result<(), JsError> {
        let cmd = Command::decode(command).map_err(|e| JsError::new(&e.to_string()))?;
        self.inner.borrow_mut().core.command(cmd, now());
        run_effects(&self.inner);
        Ok(())
    }

    /// Take all pending updates as a protobuf `neoshard.core.v1.UpdateBatch` (empty if none).
    pub fn drain(&self) -> Vec<u8> {
        let mut i = self.inner.borrow_mut();
        if !i.core.has_updates() {
            return Vec::new();
        }
        i.core.drain_updates().encode_to_vec()
    }

    /// `cb(Uint8Array)` is called with a `CacheSnapshot` to persist.
    #[wasm_bindgen(js_name = onPersist)]
    pub fn on_persist(&self, cb: js_sys::Function) {
        self.inner.borrow_mut().persist = Some(cb);
    }

    /// `cb()` is called when updates are waiting (the UI then drains on the next frame).
    #[wasm_bindgen(js_name = onUpdates)]
    pub fn on_updates(&self, cb: js_sys::Function) {
        self.inner.borrow_mut().notify = Some(cb);
    }

    pub fn version() -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }
}
