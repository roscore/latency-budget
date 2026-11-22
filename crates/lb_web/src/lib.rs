//! Browser front end: DOM wiring, Canvas 2D timeline, i18n, CSV export.

use std::cell::RefCell;

use lb_core::{clean, fmt, Budget, Preset, Report, Stage, Status, MAX_STAGES, MAX_WORKERS};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{
    CanvasRenderingContext2d as Ctx, Document, Element, Event, HtmlCanvasElement, HtmlElement,
    HtmlInputElement, KeyboardEvent, MouseEvent,
};

const COLORS: [&str; 8] =
    ["#5eb1ff", "#7ee0c3", "#b79bff", "#ffb36b", "#ff7fa8", "#9fd86b", "#6be2f0", "#e8d36b"];

/// Default stage names in (English, Korean).
const NAMES: [(&str, &str); 6] = [
    ("Sensor capture", "센서 캡처"),
    ("Decode", "디코드"),
    ("Perception", "인지"),
    ("Policy", "정책"),
    ("Safety filter", "안전 필터"),
    ("Actuation", "구동"),
];

#[derive(Clone, Copy, PartialEq)]
enum Lang {
    Ko,
    En,
}

struct App {
    budget: Budget,
    lang: Lang,
    focus: Option<usize>,
    hits: Vec<(f64, f64, f64, f64)>,
}

thread_local! {
    static APP: RefCell<App> = RefCell::new(App {
        budget: Preset::VisionControl.budget(),
        lang: Lang::Ko,
        focus: None,
        hits: Vec::new(),
    });
}

fn tr(lang: Lang, key: &str) -> &'static str {
    let (ko, en) = match key {
        "name" => ("이름", "Name"),
        "typ" => ("일반(ms)", "Typical (ms)"),
        "p95" => ("p95(ms)", "p95 (ms)"),
        "typw" => ("일반", "Typical"),
        "workers" => ("워커", "Workers"),
        "up" => ("위로 이동", "Move up"),
        "down" => ("아래로 이동", "Move down"),
        "del" => ("삭제", "Remove"),
        "p95low" => ("p95는 일반 값 이상이어야 해요", "p95 must be ≥ typical"),
        "pass" => ("통과", "PASS"),
        "warn" => ("주의", "WARN"),
        "fail" => ("실패", "FAIL"),
        "total" => ("순차 합계", "Sequential"),
        "rate" => ("최대 처리율", "Max rate"),
        "margin" => ("마진", "Margin"),
        "bneck" => ("병목", "Bottleneck"),
        "share" => ("순차 지연 중", "of sequential"),
        "deadline" => ("데드라인", "Deadline"),
        "eff" => ("유효", "effective"),
        "nolimit" => ("제한 없음", "unbounded"),
        "stage" => ("단계", "Stage"),
        "new" => ("새 단계", "New stage"),
        "empty" => ("단계를 추가해 주세요", "Add a stage to begin"),
        _ => ("", ""),
    };
    if lang == Lang::Ko { ko } else { en }
}

fn doc() -> Document {
    web_sys::window().unwrap().document().unwrap()
}

fn el(id: &str) -> Option<Element> {
    doc().get_element_by_id(id)
}

fn set_text(id: &str, text: &str) {
    if let Some(e) = el(id) {
        e.set_text_content(Some(text));
    }
}

fn input(id: &str) -> Option<HtmlInputElement> {
    el(id).and_then(|e| e.dyn_into().ok())
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Attach a listener to the element with `id` (no-op if missing). The closure lives forever.
fn on<E: wasm_bindgen::convert::FromWasmAbi + 'static>(id: &str, ty: &str, f: impl FnMut(E) + 'static) {
    let Some(target) = el(id) else { return };
    let cb = Closure::<dyn FnMut(E)>::new(f);
    target.add_event_listener_with_callback(ty, cb.as_ref().unchecked_ref()).unwrap();
    cb.forget();
}

fn localize_name(name: &str, lang: Lang) -> String {
    NAMES
        .iter()
        .find(|(en, ko)| name == *en || name == *ko)
        .map(|(en, ko)| if lang == Lang::Ko { ko } else { en }.to_string())
        .unwrap_or_else(|| name.to_string())
}

#[wasm_bindgen(start)]
pub fn start() {
    load_preset(Preset::VisionControl);
    for p in Preset::ALL {
        on(&format!("preset-{}", p.key()), "click", move |_: Event| load_preset(p));
    }
    on("lang", "click", |_: Event| {
        APP.with(|a| {
            let mut a = a.borrow_mut();
            a.lang = if a.lang == Lang::Ko { Lang::En } else { Lang::Ko };
            let lang = a.lang;
            a.budget.stages.iter_mut().for_each(|s| s.name = localize_name(&s.name, lang));
        });
        apply_lang();
        render_rows();
        update();
    });
    on("add", "click", |_: Event| {
        APP.with(|a| {
            let mut a = a.borrow_mut();
            let n = a.budget.stages.len();
            if n < MAX_STAGES {
                let name = format!("{} {}", tr(a.lang, "new"), n + 1);
                a.budget.stages.push(Stage::new(&name, 2.0, 3.0, 1));
            }
        });
        render_rows();
        update();
    });
    on("export", "click", |_: Event| export_csv());
    on("hint-close", "click", |_: Event| {
        el("hint").map(|h| h.set_attribute("hidden", ""));
    });
    for id in ["hz", "deadline"] {
        on(id, "input", move |_: Event| {
            let v = clean(input(id).map_or(0.0, |i| i.value_as_number()));
            APP.with(|a| {
                let b = &mut a.borrow_mut().budget;
                if id == "hz" { b.target_hz = v } else { b.deadline_ms = v }
            });
            update();
        });
    }
    on("rows", "input", |e: Event| on_row_input(&e, false));
    on("rows", "change", |e: Event| on_row_input(&e, true));
    on("rows", "click", |e: Event| on_row_click(&e));
    on("rows", "focusin", |e: Event| set_focus(row_index(&e)));
    on("rows", "focusout", |_: Event| set_focus(None));
    on("timeline", "mousemove", |e: MouseEvent| {
        let (x, y) = (e.offset_x() as f64, e.offset_y() as f64);
        let hit = APP.with(|a| {
            a.borrow().hits.iter().position(|&(x0, y0, x1, y1)| x >= x0 && x <= x1 && y >= y0 && y <= y1)
        });
        set_focus(hit);
    });
    on("timeline", "mouseleave", |_: MouseEvent| set_focus(None));
    on("timeline", "blur", |_: Event| set_focus(None));
    on("timeline", "keydown", |e: KeyboardEvent| {
        let (n, cur) = APP.with(|a| (a.borrow().budget.stages.len(), a.borrow().focus));
        if n == 0 { return; }
        let next = match e.key().as_str() {
            "ArrowRight" | "ArrowDown" => Some(cur.map_or(0, |i| (i + 1) % n)),
            "ArrowLeft" | "ArrowUp" => Some(cur.map_or(n - 1, |i| (i + n - 1) % n)),
            "Escape" => None,
            _ => return,
        };
        e.prevent_default();
        set_focus(next);
    });
    let cb = Closure::<dyn FnMut()>::new(draw);
    web_sys::window().unwrap().add_event_listener_with_callback("resize", cb.as_ref().unchecked_ref()).unwrap();
    cb.forget();
}

fn load_preset(p: Preset) {
    APP.with(|a| {
        let mut a = a.borrow_mut();
        let lang = a.lang;
        a.budget = p.budget();
        for s in a.budget.stages.iter_mut() {
            s.name = localize_name(&s.name, lang);
        }
        a.focus = None;
    });
    for q in Preset::ALL {
        if let Some(b) = el(&format!("preset-{}", q.key())) {
            b.set_attribute("aria-pressed", if q == p { "true" } else { "false" }).ok();
        }
    }
    let (hz, dl) = APP.with(|a| (a.borrow().budget.target_hz, a.borrow().budget.deadline_ms));
    if let Some(i) = input("hz") { i.set_value(&fmt(hz)); }
    if let Some(i) = input("deadline") { i.set_value(&fmt(dl)); }
    apply_lang();
    render_rows();
    update();
}

fn apply_lang() {
    let lang = APP.with(|a| a.borrow().lang);
    let d = doc();
    if let Some(root) = d.document_element() {
        root.set_attribute("lang", if lang == Lang::Ko { "ko" } else { "en" }).ok();
    }
    let attr = if lang == Lang::Ko { "ko" } else { "en" };
    let nodes = d.query_selector_all("[data-ko]").unwrap();
    for i in 0..nodes.length() {
        let e: Element = nodes.item(i).unwrap().unchecked_into();
        if let Some(t) = e.get_attribute(&format!("data-{attr}")) {
            e.set_text_content(Some(&t));
        }
    }
    let nodes = d.query_selector_all("[data-ko-label]").unwrap();
    for i in 0..nodes.length() {
        let e: Element = nodes.item(i).unwrap().unchecked_into();
        if let Some(t) = e.get_attribute(&format!("data-{attr}-label")) {
            e.set_attribute("aria-label", &t).ok();
            e.set_attribute("title", &t).ok();
        }
    }
}

fn render_rows() {
    let html = APP.with(|a| {
        let a = a.borrow();
        let l = a.lang;
        let n = a.budget.stages.len();
        let mut h = String::new();
        if n == 0 {
            h.push_str(&format!("<p class=\"empty\">{}</p>", tr(l, "empty")));
        }
        for (i, s) in a.budget.stages.iter().enumerate() {
            let c = COLORS[i % COLORS.len()];
            let invalid = s.p95_ms < s.typical_ms;
            h.push_str(&format!(
                concat!(
                    "<div class=\"row\" data-i=\"{i}\" style=\"--c:{c}\">",
                    "<span class=\"chip\" aria-hidden=\"true\">{num}</span>",
                    "<label class=\"f name\"><span>{ln}</span><input data-f=\"name\" type=\"text\" maxlength=\"40\" value=\"{name}\"></label>",
                    "<label class=\"f\"><span>{lt}</span><input data-f=\"typ\" type=\"number\" min=\"0\" step=\"0.1\" inputmode=\"decimal\" value=\"{typ}\"></label>",
                    "<label class=\"f\"><span>{lp}</span><input data-f=\"p95\" type=\"number\" min=\"0\" step=\"0.1\" inputmode=\"decimal\" value=\"{p95}\"{inv} aria-describedby=\"err-{i}\"></label>",
                    "<label class=\"f w\"><span>{lw}</span><input data-f=\"workers\" type=\"number\" min=\"1\" max=\"{mw}\" step=\"1\" value=\"{wk}\"></label>",
                    "<div class=\"acts\">",
                    "<button type=\"button\" data-act=\"up\" aria-label=\"{lu} {num}\" title=\"{lu}\"{dis_up}>↑</button>",
                    "<button type=\"button\" data-act=\"down\" aria-label=\"{ld} {num}\" title=\"{ld}\"{dis_dn}>↓</button>",
                    "<button type=\"button\" data-act=\"del\" aria-label=\"{lx} {num}\" title=\"{lx}\">×</button>",
                    "</div><p class=\"err\" id=\"err-{i}\" role=\"alert\">{err}</p></div>"
                ),
                i = i,
                c = c,
                num = i + 1,
                ln = tr(l, "name"),
                lt = tr(l, "typ"),
                lp = tr(l, "p95"),
                lw = tr(l, "workers"),
                lu = tr(l, "up"),
                ld = tr(l, "down"),
                lx = tr(l, "del"),
                mw = MAX_WORKERS,
                name = esc(&s.name),
                typ = fmt(s.typical_ms),
                p95 = fmt(s.p95_ms),
                wk = s.workers,
                inv = if invalid { " aria-invalid=\"true\"" } else { "" },
                err = if invalid { tr(l, "p95low") } else { "" },
                dis_up = if i == 0 { " disabled" } else { "" },
                dis_dn = if i + 1 == n { " disabled" } else { "" },
            ));
        }
        h
    });
    if let Some(rows) = el("rows") {
        rows.set_inner_html(&html);
    }
}

fn row_index(e: &Event) -> Option<usize> {
    let t: Element = e.target()?.dyn_into().ok()?;
    t.closest(".row").ok()??.get_attribute("data-i")?.parse().ok()
}

fn on_row_input(e: &Event, commit: bool) {
    let Some(i) = row_index(e) else { return };
    let Some(inp) = e.target().and_then(|t| t.dyn_into::<HtmlInputElement>().ok()) else { return };
    let field = inp.get_attribute("data-f").unwrap_or_default();
    let (lang, invalid) = APP.with(|a| {
        let mut a = a.borrow_mut();
        let lang = a.lang;
        let Some(s) = a.budget.stages.get_mut(i) else { return (lang, false) };
        let v = clean(inp.value_as_number());
        match field.as_str() {
            "name" => s.name = inp.value(),
            "typ" => s.typical_ms = v,
            "p95" => s.p95_ms = v,
            "workers" => s.workers = (v.round() as u32).clamp(1, MAX_WORKERS),
            _ => {}
        }
        if commit {
            *s = s.clone().normalized();
        }
        (lang, s.p95_ms < s.typical_ms)
    });
    if commit && field != "name" {
        // Write normalized numbers back without rebuilding the row (keeps focus).
        APP.with(|a| {
            let a = a.borrow();
            let s = &a.budget.stages[i];
            let row = inp.closest(".row").ok().flatten();
            for (f, v) in [("typ", fmt(s.typical_ms)), ("p95", fmt(s.p95_ms)), ("workers", s.workers.to_string())] {
                if let Some(x) = row.as_ref().and_then(|r| r.query_selector(&format!("[data-f={f}]")).ok().flatten()) {
                    x.unchecked_into::<HtmlInputElement>().set_value(&v);
                }
            }
        });
    }
    if let Some(row) = inp.closest(".row").ok().flatten() {
        if let Ok(Some(p)) = row.query_selector("[data-f=p95]") {
            if invalid { p.set_attribute("aria-invalid", "true").ok() } else { p.remove_attribute("aria-invalid").ok() };
        }
        if let Ok(Some(m)) = row.query_selector(".err") {
            m.set_text_content(Some(if invalid { tr(lang, "p95low") } else { "" }));
        }
    }
    update();
}

fn on_row_click(e: &Event) {
    let Some(t) = e.target().and_then(|t| t.dyn_into::<Element>().ok()) else { return };
    let Some(btn) = t.closest("button[data-act]").ok().flatten() else { return };
    let Some(i) = row_index(e) else { return };
    let act = btn.get_attribute("data-act").unwrap_or_default();
    let target = APP.with(|a| {
        let mut a = a.borrow_mut();
        let st = &mut a.budget.stages;
        match act.as_str() {
            "up" if i > 0 => { st.swap(i, i - 1); Some((i - 1, "up")) }
            "down" if i + 1 < st.len() => { st.swap(i, i + 1); Some((i + 1, "down")) }
            "del" if i < st.len() => {
                st.remove(i);
                let n = st.len();
                if n == 0 { None } else { Some((i.min(n - 1), "del")) }
            }
            _ => None,
        }
    });
    APP.with(|a| a.borrow_mut().focus = None);
    render_rows();
    update();
    // Restore keyboard focus to an equivalent control in the moved/next row.
    let sel = match target {
        Some((j, act)) => format!(".row[data-i=\"{j}\"] [data-act={act}]:not([disabled]), .row[data-i=\"{j}\"] [data-act=del]"),
        None => "#add".to_string(),
    };
    if let Ok(Some(x)) = doc().query_selector(&sel) {
        x.unchecked_into::<HtmlElement>().focus().ok();
    }
}

fn set_focus(i: Option<usize>) {
    let changed = APP.with(|a| {
        let mut a = a.borrow_mut();
        let changed = a.focus != i;
        a.focus = i;
        changed
    });
    if changed {
        draw();
    }
}

fn update() {
    let (r, b, lang) = APP.with(|a| {
        let a = a.borrow();
        (a.budget.report(), a.budget.normalized(), a.lang)
    });
    set_text("hud-total", &format!("{} / {} ms", fmt(r.typical_total_ms), fmt(r.p95_total_ms)));
    set_text("hud-rate", &r.max_hz.map_or(tr(lang, "nolimit").to_string(), |h| format!("{} Hz", fmt1(h))));
    set_text("hud-margin", &format!("{} / {} ms", signed(r.margin_typical_ms), signed(r.margin_p95_ms)));
    set_text("hud-target", &format!("{} {} ms · {} Hz", tr(lang, "deadline"), fmt(b.deadline_ms), fmt(b.target_hz)));
    let bn = r.bottleneck.map_or("—".to_string(), |i| {
        format!("{} · {}% {}", b.stages[i].name, fmt1(r.bottleneck_share * 100.0), tr(lang, "share"))
    });
    set_text("hud-bneck", &bn);
    if let Some(s) = el("hud-status") {
        s.set_text_content(Some(tr(lang, r.status.key())));
        s.set_class_name(&format!("status {}", r.status.key()));
    }
    if let Some(h) = el("hud") {
        h.set_attribute("data-status", r.status.key()).ok();
    }
    draw();
}

fn fmt1(v: f64) -> String {
    format!("{:.1}", v)
}

fn signed(v: f64) -> String {
    if v >= 0.0 { format!("+{}", fmt(v)) } else { fmt(v) }
}

fn draw() {
    let Some(canvas) = el("timeline").and_then(|e| e.dyn_into::<HtmlCanvasElement>().ok()) else { return };
    let w = web_sys::window().unwrap();
    let dpr = w.device_pixel_ratio().max(1.0);
    let cw = canvas.client_width().max(1) as f64;
    let ch = canvas.client_height().max(1) as f64;
    let (pw, ph) = ((cw * dpr).round() as u32, (ch * dpr).round() as u32);
    if canvas.width() != pw || canvas.height() != ph {
        canvas.set_width(pw);
        canvas.set_height(ph);
    }
    let ctx: Ctx = canvas.get_context("2d").unwrap().unwrap().unchecked_into();
    ctx.set_transform(dpr, 0.0, 0.0, dpr, 0.0, 0.0).ok();
    ctx.clear_rect(0.0, 0.0, cw, ch);

    let (b, r, focus, lang) = APP.with(|a| {
        let a = a.borrow();
        (a.budget.normalized(), a.budget.report(), a.focus, a.lang)
    });
    let hits = paint(&ctx, &b, &r, focus, lang, cw, ch);
    APP.with(|a| a.borrow_mut().hits = hits);
    tooltip(&b, &r, focus, lang);
}

fn paint(ctx: &Ctx, b: &Budget, r: &Report, focus: Option<usize>, lang: Lang, cw: f64, ch: f64) -> Vec<(f64, f64, f64, f64)> {
    let n = b.stages.len();
    let compact = cw < 560.0;
    let (left, right, top, bottom) = (if compact { 12.0 } else { 140.0 }, 20.0, 24.0, 30.0);
    let span = (r.p95_total_ms.max(b.deadline_ms) * 1.08).max(1.0);
    let x = |ms: f64| left + (ms / span) * (cw - left - right);
    let lane_h = ((ch - top - bottom) / n.max(1) as f64).min(46.0);
    let bar_h = (lane_h * 0.62).max(8.0);

    // Grid and axis ticks.
    let step = nice_step(span / if compact { 4.0 } else { 8.0 });
    ctx.set_font("11px system-ui, sans-serif");
    ctx.set_line_width(1.0);
    let mut t = 0.0;
    while t <= span {
        let gx = x(t).round() + 0.5;
        ctx.set_stroke_style_str("rgba(150,170,210,0.10)");
        ctx.begin_path();
        ctx.move_to(gx, top - 8.0);
        ctx.line_to(gx, ch - bottom);
        ctx.stroke();
        ctx.set_fill_style_str("rgba(180,195,225,0.7)");
        ctx.fill_text(&format!("{} ms", fmt(t)), gx + 3.0, ch - bottom + 16.0).ok();
        t += step;
    }
    // Over-deadline zone.
    let dx = x(b.deadline_ms);
    ctx.set_fill_style_str("rgba(255,90,110,0.07)");
    ctx.fill_rect(dx, top - 8.0, cw - right - dx, ch - bottom - top + 8.0);

    let mut hits = Vec::with_capacity(n);
    let (mut acc_t, mut acc_p) = (0.0, 0.0);
    for (i, s) in b.stages.iter().enumerate() {
        let c = COLORS[i % COLORS.len()];
        let y = top + i as f64 * lane_h + (lane_h - bar_h) / 2.0;
        let is_b = r.bottleneck == Some(i);
        let dim = focus.is_some_and(|f| f != i);
        ctx.set_global_alpha(if dim { 0.35 } else { 1.0 });
        // p95 overlay: where the stage would sit if every earlier stage hit p95.
        let (px0, px1) = (x(acc_p), x(acc_p + s.p95_ms));
        ctx.set_fill_style_str(c);
        ctx.set_global_alpha(if dim { 0.12 } else { 0.28 });
        ctx.fill_rect(px0, y + bar_h * 0.55, (px1 - px0).max(1.0), bar_h * 0.45);
        // Typical block.
        ctx.set_global_alpha(if dim { 0.35 } else { 1.0 });
        let (tx0, tx1) = (x(acc_t), x(acc_t + s.typical_ms));
        let tw = (tx1 - tx0).max(2.0);
        ctx.fill_rect(tx0, y, tw, bar_h * 0.55);
        if is_b {
            ctx.set_stroke_style_str("#ffe27a");
            ctx.set_line_width(2.0);
            ctx.stroke_rect(tx0 - 1.0, y - 1.0, tw + 2.0, bar_h * 0.55 + 2.0);
        }
        if focus == Some(i) {
            ctx.set_stroke_style_str("#ffffff");
            ctx.set_line_width(1.5);
            ctx.stroke_rect(tx0 - 3.0, y - 3.0, (px1.max(tx1) - tx0) + 6.0, bar_h + 6.0);
        }
        ctx.set_global_alpha(if dim { 0.5 } else { 1.0 });
        ctx.set_fill_style_str("#dfe7f7");
        let label = format!("{}{}", if is_b { "★ " } else { "" }, s.name);
        if compact {
            ctx.fill_text(&label, 4.0 + tx0.min(cw * 0.55), y - 2.0).ok();
        } else {
            ctx.fill_text(&truncate(&label, 18), 10.0, y + bar_h * 0.55).ok();
        }
        ctx.set_global_alpha(1.0);
        hits.push((tx0.min(px0) - 4.0, y - 4.0, tx1.max(px1) + 4.0, y + bar_h + 4.0));
        acc_t += s.typical_ms;
        acc_p += s.p95_ms;
    }
    // Deadline marker.
    ctx.set_stroke_style_str(if r.status == Status::Fail { "#ff6b81" } else { "#ffd166" });
    ctx.set_line_width(2.0);
    ctx.set_line_dash(&js_sys::Array::of2(&6.0.into(), &4.0.into())).ok();
    ctx.begin_path();
    ctx.move_to(dx, top - 12.0);
    ctx.line_to(dx, ch - bottom);
    ctx.stroke();
    ctx.set_line_dash(&js_sys::Array::new()).ok();
    ctx.set_fill_style_str("#ffd166");
    let dl = format!("{} {} ms", tr(lang, "deadline"), fmt(b.deadline_ms));
    let tw = ctx.measure_text(&dl).map(|m| m.width()).unwrap_or(60.0);
    ctx.fill_text(&dl, (dx - tw - 6.0).max(left), top - 12.0).ok();
    hits
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() > n { format!("{}…", s.chars().take(n - 1).collect::<String>()) } else { s.to_string() }
}

fn nice_step(raw: f64) -> f64 {
    let p = 10f64.powf(raw.max(1e-6).log10().floor());
    [1.0, 2.0, 5.0, 10.0].iter().map(|m| m * p).find(|v| *v >= raw).unwrap_or(10.0 * p)
}

fn tooltip(b: &Budget, r: &Report, focus: Option<usize>, lang: Lang) {
    let Some(tip) = el("tip").and_then(|e| e.dyn_into::<HtmlElement>().ok()) else { return };
    let Some(i) = focus.filter(|&i| i < b.stages.len()) else {
        tip.set_attribute("hidden", "").ok();
        return;
    };
    let s = &b.stages[i];
    let hit = APP.with(|a| a.borrow().hits.get(i).copied());
    let share = if r.typical_total_ms > 0.0 { s.typical_ms / r.typical_total_ms * 100.0 } else { 0.0 };
    let star = if r.bottleneck == Some(i) { format!(" · ★ {}", tr(lang, "bneck")) } else { String::new() };
    let lines = [
        format!("{} {}: {}{}", tr(lang, "stage"), i + 1, s.name, star),
        format!("{} {} ms · p95 {} ms", tr(lang, "typw"), fmt(s.typical_ms), fmt(s.p95_ms)),
        format!("{} × {} → {} {} ms · {}%", tr(lang, "workers"), s.workers, tr(lang, "eff"), fmt(s.effective_ms()), fmt1(share)),
    ];
    tip.set_inner_html(&format!("<strong>{}</strong><br>{}<br>{}", esc(&lines[0]), esc(&lines[1]), esc(&lines[2])));
    tip.remove_attribute("hidden").ok();
    if let Some((x0, y0, _, y1)) = hit {
        let cw = el("timeline").map_or(0, |c| c.client_width()) as f64;
        let st = tip.style();
        let left = x0.min(cw - tip.offset_width() as f64 - 8.0).max(8.0);
        st.set_property("left", &format!("{left}px")).ok();
        st.set_property("top", &format!("{}px", if y0 > 90.0 { y0 - 8.0 } else { y1 + 8.0 })).ok();
        st.set_property("transform", if y0 > 90.0 { "translateY(-100%)" } else { "none" }).ok();
    }
    set_text("live", &lines.join(". "));
}

fn export_csv() {
    let csv = APP.with(|a| a.borrow().budget.to_csv());
    let parts = js_sys::Array::of1(&JsValue::from_str(&format!("\u{feff}{csv}")));
    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type("text/csv;charset=utf-8");
    let Ok(blob) = web_sys::Blob::new_with_str_sequence_and_options(&parts, &opts) else { return };
    let Ok(url) = web_sys::Url::create_object_url_with_blob(&blob) else { return };
    let a: web_sys::HtmlAnchorElement = doc().create_element("a").unwrap().unchecked_into();
    a.set_href(&url);
    a.set_download("latency-budget.csv");
    a.click();
    web_sys::Url::revoke_object_url(&url).ok();
}
