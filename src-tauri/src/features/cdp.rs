//! 极简 CDP 客户端：只做 Runtime.evaluate + 一次 Debugger 闭包捕获。

use std::net::TcpStream;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{connect, Message, WebSocket};

pub struct Cdp {
    socket: WebSocket<MaybeTlsStream<TcpStream>>,
    next_id: u64,
}

impl Cdp {
    fn json_targets() -> Result<Vec<Value>, String> {
        let resp = ureq::get(&format!(
            "http://127.0.0.1:{}/json",
            super::injector::CDP_PORT
        ))
        .timeout(Duration::from_secs(3))
        .call()
        .map_err(|e| format!("GET /json 失败: {e}"))?;
        let body = resp
            .into_string()
            .map_err(|e| format!("读取 /json 失败: {e}"))?;
        serde_json::from_str(&body).map_err(|e| format!("解析 /json 失败: {e}"))
    }

    fn is_main_target(t: &Value) -> bool {
        t["type"].as_str() == Some("page")
            && t["url"]
                .as_str()
                .map(|u| u.ends_with("#/"))
                .unwrap_or(false)
    }

    fn connect_target(page: &Value) -> Result<Self, String> {
        let url = page["webSocketDebuggerUrl"]
            .as_str()
            .ok_or_else(|| "缺少 webSocketDebuggerUrl".to_string())?;
        let (socket, _) = connect(url).map_err(|e| format!("WebSocket 连接失败: {e}"))?;
        if let MaybeTlsStream::Plain(s) = socket.get_ref() {
            let _ = s.set_read_timeout(Some(Duration::from_secs(8)));
        }
        Ok(Self { socket, next_id: 1 })
    }

    pub fn connect() -> Result<Self, String> {
        let targets = Self::json_targets()?;
        let page = targets
            .iter()
            .find(|t| Self::is_main_target(t))
            .or_else(|| targets.iter().find(|t| t["type"].as_str() == Some("page")))
            .ok_or_else(|| "没有 page target".to_string())?;
        Self::connect_target(page)
    }

    /// 只连接游戏主页面 #/。高级开箱必须使用该页面，
    /// 否则会捕获到 panel 等子页面的 openChest，却读不到宝箱 DOM。
    pub fn connect_main() -> Result<Self, String> {
        let targets = Self::json_targets()?;
        let page = targets
            .iter()
            .find(|t| Self::is_main_target(t))
            .ok_or_else(|| "游戏主页面 #/ 尚未就绪".to_string())?;
        crate::logging::info(
            "cdp",
            format!(
                "连接主页面 target url={} title={}",
                page["url"].as_str().unwrap_or("?"),
                page["title"].as_str().unwrap_or("?")
            ),
        );
        Self::connect_target(page)
    }

    pub fn current_page_url(&mut self) -> Result<String, String> {
        self.evaluate("location.href", false)?
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| "无法读取当前页面 URL".to_string())
    }

    fn send_raw(&mut self, method: &str, params: Value) -> Result<u64, String> {
        let id = self.next_id;
        self.next_id += 1;
        let msg = json!({"id": id, "method": method, "params": params});
        self.socket
            .send(Message::Text(msg.to_string()))
            .map_err(|e| format!("CDP send 失败: {e}"))?;
        Ok(id)
    }

    fn read_json(&mut self) -> Result<Value, String> {
        loop {
            match self.socket.read() {
                Ok(Message::Text(t)) => {
                    return serde_json::from_str(&t).map_err(|e| format!("CDP JSON 失败: {e}"))
                }
                Ok(Message::Binary(b)) => {
                    if let Ok(v) = serde_json::from_slice::<Value>(&b) {
                        return Ok(v);
                    }
                }
                Ok(Message::Ping(p)) => {
                    let _ = self.socket.send(Message::Pong(p));
                }
                Ok(Message::Close(_)) => return Err("CDP WebSocket 已关闭".into()),
                Ok(_) => {}
                Err(e) => return Err(format!("CDP read 失败: {e}")),
            }
        }
    }

    fn send_recv(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.send_raw(method, params)?;
        let deadline = Instant::now() + Duration::from_secs(8);
        while Instant::now() < deadline {
            let v = self.read_json()?;
            if v["id"].as_u64() == Some(id) {
                return Ok(v);
            }
        }
        Err(format!("等待 CDP {method} 响应超时"))
    }

    fn read_until(&mut self, method: &str, timeout: Duration) -> Result<Value, String> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let v = self.read_json()?;
            if v["method"].as_str() == Some(method) {
                return Ok(v);
            }
        }
        Err(format!("等待 CDP 事件 {method} 超时"))
    }

    pub fn evaluate(&mut self, expr: &str, await_promise: bool) -> Result<Value, String> {
        let r = self.send_recv(
            "Runtime.evaluate",
            json!({
                "expression": expr,
                "returnByValue": true,
                "awaitPromise": await_promise,
            }),
        )?;
        Ok(r["result"]["result"]["value"].clone())
    }

    /// 读取宝箱/计数状态。锁屏与 F9 隐藏时同样可用。
    pub fn read_state(&mut self) -> Result<Value, String> {
        self.evaluate(
            r#"(() => {
  const app = document.querySelector('#app')?.__vue_app__;
  const s = app?.config?.globalProperties?.$pinia?._s?.get('GlobalState');
  const u = s?.userInfo || {};
  const now = Date.now();
  const left = Number(u.left_treasure_chest) || 0;
  const right = Number(u.right_treasure_chest) || 0;
  return {
    ok: !!s,
    now,
    left, right,
    leftIn: left - now,
    rightIn: right - now,
    cumulative: Number(s?.cumulativeClickCount) || 0,
    totalClickCount: Number(s?.totalClickCount) || 0,
    leftBtn: !!document.querySelector('.reward-anchor--left .reward-btn'),
    rightBtn: !!document.querySelector('.reward-anchor--right .reward-btn'),
    rewardBtn: !!document.querySelector('.reward-btn'),
    chestrFn: !!window.__chest?.openChest,
  };
})()"#,
            false,
        )
    }

    /// 用 Debugger 在根组件 render 的闭包里抓一次 openChest，挂到 window.__chest。
    pub fn capture_open_chest(&mut self) -> Result<(), String> {
        self.send_recv("Runtime.enable", json!({}))?;
        self.send_recv("Debugger.enable", json!({}))?;

        let mut breakpoint_id: Option<String> = None;
        let result = (|| -> Result<(), String> {
            let fn_eval = self.send_recv(
                "Runtime.evaluate",
                json!({
                    "expression": "document.querySelector('#app').__vue_app__._container._vnode.component.render",
                    "returnByValue": false,
                }),
            )?;
            let object_id = fn_eval["result"]["result"]["objectId"]
                .as_str()
                .ok_or_else(|| "拿不到 render objectId".to_string())?;
            let bp = self.send_recv(
                "Debugger.setBreakpointOnFunctionCall",
                json!({ "objectId": object_id }),
            )?;
            let bp_id = bp["result"]["breakpointId"]
                .as_str()
                .ok_or_else(|| "设置断点失败".to_string())?
                .to_string();
            breakpoint_id = Some(bp_id.clone());

            // 不等待响应：render 执行后会被断点暂停，响应要等 resume 才回
            self.send_raw(
                "Runtime.evaluate",
                json!({
                    "expression": "document.querySelector('#app').__vue_app__._container._vnode.component.update()",
                    "returnByValue": true,
                }),
            )?;

            let paused = self.read_until("Debugger.paused", Duration::from_secs(5))?;
            let frame_id = paused["params"]["callFrames"][0]["callFrameId"]
                .as_str()
                .ok_or_else(|| "paused 事件里没有 callFrameId".to_string())?;

            let expr = r#"(() => {
  const w = window;
  w.__chest = {};
  const names = ['pc','zl','openChest','jsonpClient','zr','jr','Wn','Gn'];
  for (const n of names) { try { w.__chest[n] = eval(n); } catch (e) {} }
  return Object.keys(w.__chest).map(k => k + ':' + typeof w.__chest[k]);
})()"#;
            self.send_recv(
                "Debugger.evaluateOnCallFrame",
                json!({
                    "callFrameId": frame_id,
                    "expression": expr,
                    "returnByValue": true,
                }),
            )?;

            let _ = self.send_recv(
                "Debugger.removeBreakpoint",
                json!({ "breakpointId": bp_id }),
            );
            breakpoint_id = None;
            let _ = self.send_recv("Debugger.resume", json!({}));
            Ok(())
        })();

        if result.is_err() {
            // 捕获失败时不要留下断点或暂停状态，否则官方页面可能卡住。
            if let Some(id) = breakpoint_id.as_deref() {
                let _ = self.send_recv(
                    "Debugger.removeBreakpoint",
                    json!({ "breakpointId": id }),
                );
            }
            let _ = self.send_recv("Debugger.resume", json!({}));
            let _ = self.send_recv("Debugger.disable", json!({}));
        }
        result
    }

    /// 在官方 index.render 闭包里捕获房间成员 ref 与位置缓存查找函数。
    /// 捕获后页面脚本可以写官方位置缓存，不再依赖 CSS 覆盖。
    fn capture_room_api_inner(&mut self) -> Result<(), String> {
        if self
            .evaluate("!!window.__autotappoRoomApi", false)?
            .as_bool()
            .unwrap_or(false)
        {
            return Ok(());
        }
        self.send_recv("Runtime.enable", json!({}))?;
        self.send_recv("Debugger.enable", json!({}))?;

        let fn_eval = self.send_recv(
            "Runtime.evaluate",
            json!({
                "expression": "(()=>{const a=document.querySelector('#app')?.__vue_app__;const c=a?._container?._vnode?.component;const r=c?.subTree?.component;return r?.subTree?.component?.render})()",
                "returnByValue": false,
            }),
        )?;
        let object_id = fn_eval["result"]["result"]["objectId"]
            .as_str()
            .ok_or_else(|| "拿不到房间 render objectId".to_string())?
            .to_string();
        let bp1 = self.send_recv(
            "Debugger.setBreakpointOnFunctionCall",
            json!({ "objectId": object_id }),
        )?;
        let mut bp_id = bp1["result"]["breakpointId"]
            .as_str()
            .ok_or_else(|| "设置房间断点失败".to_string())?
            .to_string();

        self.send_raw(
            "Runtime.evaluate",
            json!({
                "expression": "(()=>{const a=document.querySelector('#app')?.__vue_app__;const c=a?._container?._vnode?.component;c?.subTree?.component?.update();return 1})()",
                "returnByValue": true,
            }),
        )?;
        let mut paused = self.read_until("Debugger.paused", Duration::from_secs(5))?;
        let mut frame_id = paused["params"]["callFrames"][0]["callFrameId"]
            .as_str()
            .ok_or_else(|| "房间 paused 缺少 callFrameId".to_string())?
            .to_string();

        let kind = self.send_recv(
            "Debugger.evaluateOnCallFrame",
            json!({
                "callFrameId": frame_id,
                "expression": "typeof pt",
                "returnByValue": true,
            }),
        )?;
        let has_pt = kind["result"]["result"]["value"].as_str() == Some("object");
        if !has_pt {
            // AutoTappo 包装层：先取 state.orig，再在官方 render 上断点。
            let orig = self.send_recv(
                "Debugger.evaluateOnCallFrame",
                json!({
                    "callFrameId": frame_id,
                    "expression": "state.orig",
                    "returnByValue": false,
                }),
            )?;
            let orig_id = orig["result"]["result"]["objectId"]
                .as_str()
                .ok_or_else(|| "拿不到官方 render objectId".to_string())?
                .to_string();
            let _ = self.send_recv(
                "Debugger.removeBreakpoint",
                json!({ "breakpointId": bp_id.clone() }),
            );
            let bp2 = self.send_recv(
                "Debugger.setBreakpointOnFunctionCall",
                json!({ "objectId": orig_id }),
            )?;
            bp_id = bp2["result"]["breakpointId"]
                .as_str()
                .ok_or_else(|| "设置官方断点失败".to_string())?
                .to_string();
            let _ = self.send_recv("Debugger.resume", json!({}));
            paused = self.read_until("Debugger.paused", Duration::from_secs(5))?;
            frame_id = paused["params"]["callFrames"][0]["callFrameId"]
                .as_str()
                .ok_or_else(|| "官方 paused 缺少 callFrameId".to_string())?
                .to_string();
        }

        let cap = self.send_recv(
            "Debugger.evaluateOnCallFrame",
            json!({
                "callFrameId": frame_id,
                "expression": "(()=>{if(typeof pt==='undefined'||typeof Vi!=='function')return 'missing';window.__autotappoRoomApi={members:pt,lookup:Vi};return 'ok'})()",
                "returnByValue": true,
            }),
        )?;
        let ok = cap["result"]["result"]["value"].as_str() == Some("ok");

        let _ = self.send_recv(
            "Debugger.removeBreakpoint",
            json!({ "breakpointId": bp_id }),
        );
        let _ = self.send_recv("Debugger.resume", json!({}));
        if ok { Ok(()) } else { Err("未能捕获房间成员 ref".into()) }
    }

    /// 捕获失败时恢复 Debugger，避免官方页面停在断点处。
    pub fn capture_room_api(&mut self) -> Result<(), String> {
        let result = self.capture_room_api_inner();
        if result.is_err() {
            let _ = self.send_recv("Debugger.resume", json!({}));
            let _ = self.send_recv("Debugger.disable", json!({}));
        }
        result
    }

    /// 调用官方 openChest。click_num 必须取调用瞬间的 cumulativeClickCount，
    /// 不能取 Rust 侧稍早读到的快照，否则服务端会报「客户端点击数异常」。
    pub fn open_chest(&mut self, place: &str) -> Result<Value, String> {
        let expr = format!(
            r#"(async () => {{
  const f = window.__chest?.openChest;
  if (!f) return {{ ok: false, err: 'openChest 未捕获' }};
  const app = document.querySelector('#app')?.__vue_app__;
  const s = app?.config?.globalProperties?.$pinia?._s?.get('GlobalState');
  const click_num = Number(s?.cumulativeClickCount) || 0;
  if (!click_num) return {{ ok: false, err: '客户端点击数异常' }};
  try {{
    const r = await f({{ place: {:?}, click_num }});
    // 直接调用 openChest 不会经过官方 zl()，必须把服务端返回的冷却时间写回 Pinia，
    // 否则隐藏状态下 reward-btn 会残留，下一步会误判为还可开。
    const d = r?.data || {{}};
    const norm = (v) => {{
      const n = Number(v);
      if (!Number.isFinite(n) || n <= 0) return 0;
      return n < 1e12 ? n * 1000 : n;
    }};
    const ln = norm(d.left_next_time);
    const rn = norm(d.right_next_time);
    if (s?.userInfo && ln > Date.now()) s.userInfo.left_treasure_chest = ln;
    if (s?.userInfo && rn > Date.now()) s.userInfo.right_treasure_chest = rn;
    return {{ ok: true, result: r }};
  }} catch (e) {{
    return {{ ok: false, err: String(e) }};
  }}
}})()"#,
            place
        );
        self.evaluate(&expr, true)
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn main_target_only_accepts_root_route() {
        assert!(Cdp::is_main_target(&json!({
            "type": "page",
            "url": "http://tauri.localhost/#/"
        })));
        assert!(!Cdp::is_main_target(&json!({
            "type": "page",
            "url": "http://tauri.localhost/index.html#/panel"
        })));
        assert!(!Cdp::is_main_target(&json!({
            "type": "page",
            "url": "http://tauri.localhost/index.html#/login"
        })));
    }
}
