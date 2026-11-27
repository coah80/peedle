use crate::builtins::{set_global, set_method, set_symbol_method};
use crate::error::JsError;
use crate::heap::*;
use crate::value::*;
use crate::vm::JsRuntime;
use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveDateTime, TimeZone, Timelike, Utc};
use std::rc::Rc;

const MAX_TIME: f64 = 8.64e15;

fn time_clip(t: f64) -> f64 {
    if !t.is_finite() || t.abs() > MAX_TIME {
        f64::NAN
    } else {
        let v = t.trunc();
        if v == 0.0 { 0.0 } else { v }
    }
}

fn now_ms() -> f64 {
    Utc::now().timestamp_millis() as f64
}

fn dt_from_ms(ms: f64) -> Option<DateTime<Utc>> {
    if !ms.is_finite() {
        return None;
    }
    DateTime::<Utc>::from_timestamp_millis(ms as i64)
}

fn local_dt_from_ms(ms: f64) -> Option<DateTime<Local>> {
    dt_from_ms(ms).map(|d| d.with_timezone(&Local))
}

fn make_local_ms(y: i32, m: i32, d: i32, h: i32, min: i32, s: i32, ms: i32) -> f64 {
    let utc = make_utc_ms(y, m, d, h, min, s, ms);
    if !utc.is_finite() {
        return f64::NAN;
    }
    if let Some(dt) = dt_from_ms(utc) {
        let off = Local.offset_from_utc_datetime(&dt.naive_utc());
        let adj = utc - (off.local_minus_utc() as f64) * 1000.0;
        return time_clip(adj);
    }
    time_clip(utc)
}

fn make_utc_ms(y: i32, m: i32, d: i32, h: i32, min: i32, s: i32, ms: i32) -> f64 {
    let total_months = y as i64 * 12 + m as i64;
    let year = total_months.div_euclid(12) as i32;
    let month = total_months.rem_euclid(12) as u32 + 1;
    let Some(base_date) = NaiveDate::from_ymd_opt(year, month, 1) else {
        return f64::NAN;
    };
    let day_offset = (d as i64) - 1;
    let total_ms = day_offset * 86_400_000
        + (h as i64) * 3_600_000
        + (min as i64) * 60_000
        + (s as i64) * 1_000
        + (ms as i64);
    let base_dt = base_date.and_hms_opt(0, 0, 0).unwrap().and_utc();
    time_clip((base_dt.timestamp_millis() + total_ms) as f64)
}

fn parse_date_string(s: &str) -> f64 {
    let trimmed = s.trim();
    if let Ok(dt) = DateTime::parse_from_rfc3339(trimmed) {
        return time_clip(dt.timestamp_millis() as f64);
    }
    if let Ok(dt) = DateTime::parse_from_rfc2822(trimmed) {
        return time_clip(dt.timestamp_millis() as f64);
    }
    if let Ok(nd) = NaiveDate::parse_from_str(trimmed, "%Y-%m-%d") {
        let dt = nd.and_hms_opt(0, 0, 0).unwrap().and_utc();
        return time_clip(dt.timestamp_millis() as f64);
    }
    for fmt in [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M:%S",
        "%B %d, %Y %H:%M:%S",
        "%b %d, %Y %H:%M:%S",
        "%B %d, %Y",
        "%b %d, %Y",
        "%m/%d/%Y",
    ] {
        if let Ok(ndt) = NaiveDateTime::parse_from_str(trimmed, fmt) {
            if let Some(local) = Local.from_local_datetime(&ndt).single() {
                return time_clip(local.timestamp_millis() as f64);
            }
            return time_clip(ndt.and_utc().timestamp_millis() as f64);
        }
        if let Ok(nd) = NaiveDate::parse_from_str(trimmed, fmt) {
            let dt = nd.and_hms_opt(0, 0, 0).unwrap().and_utc();
            return time_clip(dt.timestamp_millis() as f64);
        }
    }
    f64::NAN
}

fn this_time(rt: &JsRuntime, this: &JsValue) -> Result<f64, JsError> {
    let JsValue::Object(oid) = this else {
        return Err(JsError::type_err("this is not a Date object"));
    };
    match rt.heap.get_obj(*oid).kind {
        ObjectKind::Date(ms) => Ok(ms),
        _ => Err(JsError::type_err("this is not a Date object")),
    }
}

fn set_this_time(rt: &mut JsRuntime, this: &JsValue, ms: f64) -> Result<JsValue, JsError> {
    let JsValue::Object(oid) = this else {
        return Err(JsError::type_err("this is not a Date object"));
    };
    let clipped = time_clip(ms);
    if let ObjectKind::Date(ref mut cur) = rt.heap.get_obj_mut(*oid).kind {
        *cur = clipped;
        Ok(JsValue::Number(clipped))
    } else {
        Err(JsError::type_err("this is not a Date object"))
    }
}

pub fn init(rt: &mut JsRuntime) {
    let proto = rt.intrinsics.date_proto;

    set_method(
        rt,
        proto,
        "getTime",
        0,
        Rc::new(|rt, this, _| this_time(rt, &this).map(JsValue::Number)),
    );
    set_method(
        rt,
        proto,
        "valueOf",
        0,
        Rc::new(|rt, this, _| this_time(rt, &this).map(JsValue::Number)),
    );
    set_method(
        rt,
        proto,
        "setTime",
        1,
        Rc::new(|rt, this, args| {
            let t = rt.to_number(args.first().unwrap_or(&JsValue::Undefined))?;
            set_this_time(rt, &this, t)
        }),
    );

    set_method(
        rt,
        proto,
        "getUTCFullYear",
        0,
        Rc::new(|rt, this, _| {
            let ms = this_time(rt, &this)?;
            Ok(JsValue::Number(
                dt_from_ms(ms).map(|d| d.year() as f64).unwrap_or(f64::NAN),
            ))
        }),
    );
    set_method(
        rt,
        proto,
        "getUTCMonth",
        0,
        Rc::new(|rt, this, _| {
            let ms = this_time(rt, &this)?;
            Ok(JsValue::Number(
                dt_from_ms(ms)
                    .map(|d| d.month0() as f64)
                    .unwrap_or(f64::NAN),
            ))
        }),
    );
    set_method(
        rt,
        proto,
        "getUTCDate",
        0,
        Rc::new(|rt, this, _| {
            let ms = this_time(rt, &this)?;
            Ok(JsValue::Number(
                dt_from_ms(ms).map(|d| d.day() as f64).unwrap_or(f64::NAN),
            ))
        }),
    );
    set_method(
        rt,
        proto,
        "getUTCDay",
        0,
        Rc::new(|rt, this, _| {
            let ms = this_time(rt, &this)?;
            Ok(JsValue::Number(
                dt_from_ms(ms)
                    .map(|d| d.weekday().num_days_from_sunday() as f64)
                    .unwrap_or(f64::NAN),
            ))
        }),
    );
    set_method(
        rt,
        proto,
        "getUTCHours",
        0,
        Rc::new(|rt, this, _| {
            let ms = this_time(rt, &this)?;
            Ok(JsValue::Number(
                dt_from_ms(ms).map(|d| d.hour() as f64).unwrap_or(f64::NAN),
            ))
        }),
    );
    set_method(
        rt,
        proto,
        "getUTCMinutes",
        0,
        Rc::new(|rt, this, _| {
            let ms = this_time(rt, &this)?;
            Ok(JsValue::Number(
                dt_from_ms(ms)
                    .map(|d| d.minute() as f64)
                    .unwrap_or(f64::NAN),
            ))
        }),
    );
    set_method(
        rt,
        proto,
        "getUTCSeconds",
        0,
        Rc::new(|rt, this, _| {
            let ms = this_time(rt, &this)?;
            Ok(JsValue::Number(
                dt_from_ms(ms)
                    .map(|d| d.second() as f64)
                    .unwrap_or(f64::NAN),
            ))
        }),
    );
    set_method(
        rt,
        proto,
        "getUTCMilliseconds",
        0,
        Rc::new(|rt, this, _| {
            let ms = this_time(rt, &this)?;
            Ok(JsValue::Number(
                dt_from_ms(ms)
                    .map(|d| (d.timestamp_subsec_millis() % 1000) as f64)
                    .unwrap_or(f64::NAN),
            ))
        }),
    );

    set_method(
        rt,
        proto,
        "getFullYear",
        0,
        Rc::new(|rt, this, _| {
            let ms = this_time(rt, &this)?;
            Ok(JsValue::Number(
                local_dt_from_ms(ms)
                    .map(|d| d.year() as f64)
                    .unwrap_or(f64::NAN),
            ))
        }),
    );
    set_method(
        rt,
        proto,
        "getMonth",
        0,
        Rc::new(|rt, this, _| {
            let ms = this_time(rt, &this)?;
            Ok(JsValue::Number(
                local_dt_from_ms(ms)
                    .map(|d| d.month0() as f64)
                    .unwrap_or(f64::NAN),
            ))
        }),
    );
    set_method(
        rt,
        proto,
        "getDate",
        0,
        Rc::new(|rt, this, _| {
            let ms = this_time(rt, &this)?;
            Ok(JsValue::Number(
                local_dt_from_ms(ms)
                    .map(|d| d.day() as f64)
                    .unwrap_or(f64::NAN),
            ))
        }),
    );
    set_method(
        rt,
        proto,
        "getDay",
        0,
        Rc::new(|rt, this, _| {
            let ms = this_time(rt, &this)?;
            Ok(JsValue::Number(
                local_dt_from_ms(ms)
                    .map(|d| d.weekday().num_days_from_sunday() as f64)
                    .unwrap_or(f64::NAN),
            ))
        }),
    );
    set_method(
        rt,
        proto,
        "getHours",
        0,
        Rc::new(|rt, this, _| {
            let ms = this_time(rt, &this)?;
            Ok(JsValue::Number(
                local_dt_from_ms(ms)
                    .map(|d| d.hour() as f64)
                    .unwrap_or(f64::NAN),
            ))
        }),
    );
    set_method(
        rt,
        proto,
        "getMinutes",
        0,
        Rc::new(|rt, this, _| {
            let ms = this_time(rt, &this)?;
            Ok(JsValue::Number(
                local_dt_from_ms(ms)
                    .map(|d| d.minute() as f64)
                    .unwrap_or(f64::NAN),
            ))
        }),
    );
    set_method(
        rt,
        proto,
        "getSeconds",
        0,
        Rc::new(|rt, this, _| {
            let ms = this_time(rt, &this)?;
            Ok(JsValue::Number(
                local_dt_from_ms(ms)
                    .map(|d| d.second() as f64)
                    .unwrap_or(f64::NAN),
            ))
        }),
    );
    set_method(
        rt,
        proto,
        "getMilliseconds",
        0,
        Rc::new(|rt, this, _| {
            let ms = this_time(rt, &this)?;
            Ok(JsValue::Number(
                local_dt_from_ms(ms)
                    .map(|d| (d.timestamp_subsec_millis() % 1000) as f64)
                    .unwrap_or(f64::NAN),
            ))
        }),
    );
    set_method(
        rt,
        proto,
        "getTimezoneOffset",
        0,
        Rc::new(|rt, this, _| {
            let ms = this_time(rt, &this)?;
            let off = local_dt_from_ms(ms)
                .map(|d| -(d.offset().local_minus_utc() as f64) / 60.0)
                .unwrap_or(f64::NAN);
            Ok(JsValue::Number(off))
        }),
    );

    set_method(
        rt,
        proto,
        "toISOString",
        0,
        Rc::new(|rt, this, _| {
            let ms = this_time(rt, &this)?;
            let Some(dt) = dt_from_ms(ms) else {
                return Err(JsError::range("Invalid time value"));
            };
            Ok(JsValue::String(dt.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()))
        }),
    );

    set_method(
        rt,
        proto,
        "toJSON",
        1,
        Rc::new(|rt, this, _| {
            let oid = rt.to_object(&this)?;
            let tv = rt.to_primitive(&JsValue::Object(oid), "number")?;
            if let JsValue::Number(n) = tv {
                if !n.is_finite() {
                    return Ok(JsValue::Null);
                }
            }
            let f = rt.get_property(oid, "toISOString")?;
            rt.call_function(f, JsValue::Object(oid), &[])
        }),
    );

    set_method(
        rt,
        proto,
        "toUTCString",
        0,
        Rc::new(|rt, this, _| {
            let ms = this_time(rt, &this)?;
            let Some(dt) = dt_from_ms(ms) else {
                return Ok(JsValue::String("Invalid Date".into()));
            };
            Ok(JsValue::String(
                dt.format("%a, %d %b %Y %H:%M:%S GMT").to_string(),
            ))
        }),
    );

    set_method(
        rt,
        proto,
        "toString",
        0,
        Rc::new(|rt, this, _| {
            let ms = this_time(rt, &this)?;
            let Some(dt) = local_dt_from_ms(ms) else {
                return Ok(JsValue::String("Invalid Date".into()));
            };
            Ok(JsValue::String(
                dt.format("%a %b %d %Y %H:%M:%S GMT%z").to_string(),
            ))
        }),
    );

    set_method(
        rt,
        proto,
        "toDateString",
        0,
        Rc::new(|rt, this, _| {
            let ms = this_time(rt, &this)?;
            let Some(dt) = local_dt_from_ms(ms) else {
                return Ok(JsValue::String("Invalid Date".into()));
            };
            Ok(JsValue::String(dt.format("%a %b %d %Y").to_string()))
        }),
    );

    set_method(
        rt,
        proto,
        "toTimeString",
        0,
        Rc::new(|rt, this, _| {
            let ms = this_time(rt, &this)?;
            let Some(dt) = local_dt_from_ms(ms) else {
                return Ok(JsValue::String("Invalid Date".into()));
            };
            Ok(JsValue::String(dt.format("%H:%M:%S GMT%z").to_string()))
        }),
    );

    set_method(
        rt,
        proto,
        "toLocaleString",
        0,
        Rc::new(|rt, this, _| {
            let ms = this_time(rt, &this)?;
            let Some(dt) = local_dt_from_ms(ms) else {
                return Ok(JsValue::String("Invalid Date".into()));
            };
            Ok(JsValue::String(dt.format("%m/%d/%Y, %H:%M:%S").to_string()))
        }),
    );
    set_method(
        rt,
        proto,
        "toLocaleDateString",
        0,
        Rc::new(|rt, this, _| {
            let ms = this_time(rt, &this)?;
            let Some(dt) = local_dt_from_ms(ms) else {
                return Ok(JsValue::String("Invalid Date".into()));
            };
            Ok(JsValue::String(dt.format("%m/%d/%Y").to_string()))
        }),
    );
    set_method(
        rt,
        proto,
        "toLocaleTimeString",
        0,
        Rc::new(|rt, this, _| {
            let ms = this_time(rt, &this)?;
            let Some(dt) = local_dt_from_ms(ms) else {
                return Ok(JsValue::String("Invalid Date".into()));
            };
            Ok(JsValue::String(dt.format("%H:%M:%S").to_string()))
        }),
    );

    set_method(
        rt,
        proto,
        "setMilliseconds",
        1,
        Rc::new(|rt, this, args| {
            let t = this_time(rt, &this)?;
            let ms = rt.to_number(args.first().unwrap_or(&JsValue::Undefined))?;
            if !t.is_finite() || !ms.is_finite() {
                return set_this_time(rt, &this, f64::NAN);
            }
            let Some(dt) = local_dt_from_ms(t) else {
                return set_this_time(rt, &this, f64::NAN);
            };
            let new_t = make_local_ms(
                dt.year(),
                dt.month0() as i32,
                dt.day() as i32,
                dt.hour() as i32,
                dt.minute() as i32,
                dt.second() as i32,
                ms.trunc() as i32,
            );
            set_this_time(rt, &this, new_t)
        }),
    );
    set_method(
        rt,
        proto,
        "setUTCMilliseconds",
        1,
        Rc::new(|rt, this, args| {
            let t = this_time(rt, &this)?;
            let ms = rt.to_number(args.first().unwrap_or(&JsValue::Undefined))?;
            if !t.is_finite() || !ms.is_finite() {
                return set_this_time(rt, &this, f64::NAN);
            }
            let Some(dt) = dt_from_ms(t) else {
                return set_this_time(rt, &this, f64::NAN);
            };
            let new_t = make_utc_ms(
                dt.year(),
                dt.month0() as i32,
                dt.day() as i32,
                dt.hour() as i32,
                dt.minute() as i32,
                dt.second() as i32,
                ms.trunc() as i32,
            );
            set_this_time(rt, &this, new_t)
        }),
    );
    set_method(
        rt,
        proto,
        "setSeconds",
        2,
        Rc::new(|rt, this, args| {
            let t = this_time(rt, &this)?;
            let sec = rt.to_number(args.first().unwrap_or(&JsValue::Undefined))?;
            let ms_arg = match args.get(1) {
                Some(v) => Some(rt.to_number(v)?),
                None => None,
            };
            if !t.is_finite() || !sec.is_finite() || ms_arg.is_some_and(|m| !m.is_finite()) {
                return set_this_time(rt, &this, f64::NAN);
            }
            let Some(dt) = local_dt_from_ms(t) else {
                return set_this_time(rt, &this, f64::NAN);
            };
            let ms = ms_arg.map(|m| m.trunc() as i32).unwrap_or((dt.timestamp_subsec_millis() % 1000) as i32);
            let new_t = make_local_ms(dt.year(), dt.month0() as i32, dt.day() as i32, dt.hour() as i32, dt.minute() as i32, sec.trunc() as i32, ms);
            set_this_time(rt, &this, new_t)
        }),
    );
    set_method(
        rt,
        proto,
        "setUTCSeconds",
        2,
        Rc::new(|rt, this, args| {
            let t = this_time(rt, &this)?;
            let sec = rt.to_number(args.first().unwrap_or(&JsValue::Undefined))?;
            let ms_arg = match args.get(1) {
                Some(v) => Some(rt.to_number(v)?),
                None => None,
            };
            if !t.is_finite() || !sec.is_finite() || ms_arg.is_some_and(|m| !m.is_finite()) {
                return set_this_time(rt, &this, f64::NAN);
            }
            let Some(dt) = dt_from_ms(t) else {
                return set_this_time(rt, &this, f64::NAN);
            };
            let ms = ms_arg.map(|m| m.trunc() as i32).unwrap_or((dt.timestamp_subsec_millis() % 1000) as i32);
            let new_t = make_utc_ms(dt.year(), dt.month0() as i32, dt.day() as i32, dt.hour() as i32, dt.minute() as i32, sec.trunc() as i32, ms);
            set_this_time(rt, &this, new_t)
        }),
    );
    set_method(
        rt,
        proto,
        "setMinutes",
        3,
        Rc::new(|rt, this, args| {
            let t = this_time(rt, &this)?;
            let min = rt.to_number(args.first().unwrap_or(&JsValue::Undefined))?;
            let sec_arg = match args.get(1) { Some(v) => Some(rt.to_number(v)?), None => None };
            let ms_arg = match args.get(2) { Some(v) => Some(rt.to_number(v)?), None => None };
            if !t.is_finite() || !min.is_finite() || sec_arg.is_some_and(|x| !x.is_finite()) || ms_arg.is_some_and(|x| !x.is_finite()) {
                return set_this_time(rt, &this, f64::NAN);
            }
            let Some(dt) = local_dt_from_ms(t) else { return set_this_time(rt, &this, f64::NAN); };
            let sec = sec_arg.map(|x| x.trunc() as i32).unwrap_or(dt.second() as i32);
            let ms = ms_arg.map(|x| x.trunc() as i32).unwrap_or((dt.timestamp_subsec_millis() % 1000) as i32);
            let new_t = make_local_ms(dt.year(), dt.month0() as i32, dt.day() as i32, dt.hour() as i32, min.trunc() as i32, sec, ms);
            set_this_time(rt, &this, new_t)
        }),
    );
    set_method(
        rt,
        proto,
        "setUTCMinutes",
        3,
        Rc::new(|rt, this, args| {
            let t = this_time(rt, &this)?;
            let min = rt.to_number(args.first().unwrap_or(&JsValue::Undefined))?;
            let sec_arg = match args.get(1) { Some(v) => Some(rt.to_number(v)?), None => None };
            let ms_arg = match args.get(2) { Some(v) => Some(rt.to_number(v)?), None => None };
            if !t.is_finite() || !min.is_finite() || sec_arg.is_some_and(|x| !x.is_finite()) || ms_arg.is_some_and(|x| !x.is_finite()) {
                return set_this_time(rt, &this, f64::NAN);
            }
            let Some(dt) = dt_from_ms(t) else { return set_this_time(rt, &this, f64::NAN); };
            let sec = sec_arg.map(|x| x.trunc() as i32).unwrap_or(dt.second() as i32);
            let ms = ms_arg.map(|x| x.trunc() as i32).unwrap_or((dt.timestamp_subsec_millis() % 1000) as i32);
            let new_t = make_utc_ms(dt.year(), dt.month0() as i32, dt.day() as i32, dt.hour() as i32, min.trunc() as i32, sec, ms);
            set_this_time(rt, &this, new_t)
        }),
    );
    set_method(
        rt,
        proto,
        "setHours",
        4,
        Rc::new(|rt, this, args| {
            let t = this_time(rt, &this)?;
            let hr = rt.to_number(args.first().unwrap_or(&JsValue::Undefined))?;
            let min_arg = match args.get(1) { Some(v) => Some(rt.to_number(v)?), None => None };
            let sec_arg = match args.get(2) { Some(v) => Some(rt.to_number(v)?), None => None };
            let ms_arg = match args.get(3) { Some(v) => Some(rt.to_number(v)?), None => None };
            if !t.is_finite() || !hr.is_finite() || min_arg.is_some_and(|x| !x.is_finite()) || sec_arg.is_some_and(|x| !x.is_finite()) || ms_arg.is_some_and(|x| !x.is_finite()) {
                return set_this_time(rt, &this, f64::NAN);
            }
            let Some(dt) = local_dt_from_ms(t) else { return set_this_time(rt, &this, f64::NAN); };
            let min = min_arg.map(|x| x.trunc() as i32).unwrap_or(dt.minute() as i32);
            let sec = sec_arg.map(|x| x.trunc() as i32).unwrap_or(dt.second() as i32);
            let ms = ms_arg.map(|x| x.trunc() as i32).unwrap_or((dt.timestamp_subsec_millis() % 1000) as i32);
            let new_t = make_local_ms(dt.year(), dt.month0() as i32, dt.day() as i32, hr.trunc() as i32, min, sec, ms);
            set_this_time(rt, &this, new_t)
        }),
    );
    set_method(
        rt,
        proto,
        "setUTCHours",
        4,
        Rc::new(|rt, this, args| {
            let t = this_time(rt, &this)?;
            let hr = rt.to_number(args.first().unwrap_or(&JsValue::Undefined))?;
            let min_arg = match args.get(1) { Some(v) => Some(rt.to_number(v)?), None => None };
            let sec_arg = match args.get(2) { Some(v) => Some(rt.to_number(v)?), None => None };
            let ms_arg = match args.get(3) { Some(v) => Some(rt.to_number(v)?), None => None };
            if !t.is_finite() || !hr.is_finite() || min_arg.is_some_and(|x| !x.is_finite()) || sec_arg.is_some_and(|x| !x.is_finite()) || ms_arg.is_some_and(|x| !x.is_finite()) {
                return set_this_time(rt, &this, f64::NAN);
            }
            let Some(dt) = dt_from_ms(t) else { return set_this_time(rt, &this, f64::NAN); };
            let min = min_arg.map(|x| x.trunc() as i32).unwrap_or(dt.minute() as i32);
            let sec = sec_arg.map(|x| x.trunc() as i32).unwrap_or(dt.second() as i32);
            let ms = ms_arg.map(|x| x.trunc() as i32).unwrap_or((dt.timestamp_subsec_millis() % 1000) as i32);
            let new_t = make_utc_ms(dt.year(), dt.month0() as i32, dt.day() as i32, hr.trunc() as i32, min, sec, ms);
            set_this_time(rt, &this, new_t)
        }),
    );
    set_method(
        rt,
        proto,
        "setDate",
        1,
        Rc::new(|rt, this, args| {
            let t = this_time(rt, &this)?;
            let d = rt.to_number(args.first().unwrap_or(&JsValue::Undefined))?;
            if !t.is_finite() || !d.is_finite() { return set_this_time(rt, &this, f64::NAN); }
            let Some(dt) = local_dt_from_ms(t) else { return set_this_time(rt, &this, f64::NAN); };
            let new_t = make_local_ms(dt.year(), dt.month0() as i32, d.trunc() as i32, dt.hour() as i32, dt.minute() as i32, dt.second() as i32, (dt.timestamp_subsec_millis() % 1000) as i32);
            set_this_time(rt, &this, new_t)
        }),
    );
    set_method(
        rt,
        proto,
        "setUTCDate",
        1,
        Rc::new(|rt, this, args| {
            let t = this_time(rt, &this)?;
            let d = rt.to_number(args.first().unwrap_or(&JsValue::Undefined))?;
            if !t.is_finite() || !d.is_finite() { return set_this_time(rt, &this, f64::NAN); }
            let Some(dt) = dt_from_ms(t) else { return set_this_time(rt, &this, f64::NAN); };
            let new_t = make_utc_ms(dt.year(), dt.month0() as i32, d.trunc() as i32, dt.hour() as i32, dt.minute() as i32, dt.second() as i32, (dt.timestamp_subsec_millis() % 1000) as i32);
            set_this_time(rt, &this, new_t)
        }),
    );
    set_method(
        rt,
        proto,
        "setMonth",
        2,
        Rc::new(|rt, this, args| {
            let t = this_time(rt, &this)?;
            let m = rt.to_number(args.first().unwrap_or(&JsValue::Undefined))?;
            let d_arg = match args.get(1) { Some(v) => Some(rt.to_number(v)?), None => None };
            if !t.is_finite() || !m.is_finite() || d_arg.is_some_and(|x| !x.is_finite()) { return set_this_time(rt, &this, f64::NAN); }
            let Some(dt) = local_dt_from_ms(t) else { return set_this_time(rt, &this, f64::NAN); };
            let d = d_arg.map(|x| x.trunc() as i32).unwrap_or(dt.day() as i32);
            let new_t = make_local_ms(dt.year(), m.trunc() as i32, d, dt.hour() as i32, dt.minute() as i32, dt.second() as i32, (dt.timestamp_subsec_millis() % 1000) as i32);
            set_this_time(rt, &this, new_t)
        }),
    );
    set_method(
        rt,
        proto,
        "setUTCMonth",
        2,
        Rc::new(|rt, this, args| {
            let t = this_time(rt, &this)?;
            let m = rt.to_number(args.first().unwrap_or(&JsValue::Undefined))?;
            let d_arg = match args.get(1) { Some(v) => Some(rt.to_number(v)?), None => None };
            if !t.is_finite() || !m.is_finite() || d_arg.is_some_and(|x| !x.is_finite()) { return set_this_time(rt, &this, f64::NAN); }
            let Some(dt) = dt_from_ms(t) else { return set_this_time(rt, &this, f64::NAN); };
            let d = d_arg.map(|x| x.trunc() as i32).unwrap_or(dt.day() as i32);
            let new_t = make_utc_ms(dt.year(), m.trunc() as i32, d, dt.hour() as i32, dt.minute() as i32, dt.second() as i32, (dt.timestamp_subsec_millis() % 1000) as i32);
            set_this_time(rt, &this, new_t)
        }),
    );
    set_method(
        rt,
        proto,
        "setFullYear",
        3,
        Rc::new(|rt, this, args| {
            let t = this_time(rt, &this)?;
            let y = rt.to_number(args.first().unwrap_or(&JsValue::Undefined))?;
            let m_arg = match args.get(1) { Some(v) => Some(rt.to_number(v)?), None => None };
            let d_arg = match args.get(2) { Some(v) => Some(rt.to_number(v)?), None => None };
            if !y.is_finite() || m_arg.is_some_and(|x| !x.is_finite()) || d_arg.is_some_and(|x| !x.is_finite()) {
                return set_this_time(rt, &this, f64::NAN);
            }
            let base = if t.is_finite() { t } else { 0.0 };
            let Some(dt) = local_dt_from_ms(base) else { return set_this_time(rt, &this, f64::NAN); };
            let m = m_arg.map(|x| x.trunc() as i32).unwrap_or(if t.is_finite() { dt.month0() as i32 } else { 0 });
            let d = d_arg.map(|x| x.trunc() as i32).unwrap_or(if t.is_finite() { dt.day() as i32 } else { 1 });
            let new_t = make_local_ms(y.trunc() as i32, m, d, if t.is_finite() { dt.hour() as i32 } else { 0 }, if t.is_finite() { dt.minute() as i32 } else { 0 }, if t.is_finite() { dt.second() as i32 } else { 0 }, if t.is_finite() { (dt.timestamp_subsec_millis() % 1000) as i32 } else { 0 });
            set_this_time(rt, &this, new_t)
        }),
    );
    set_method(
        rt,
        proto,
        "setUTCFullYear",
        3,
        Rc::new(|rt, this, args| {
            let t = this_time(rt, &this)?;
            let y = rt.to_number(args.first().unwrap_or(&JsValue::Undefined))?;
            let m_arg = match args.get(1) { Some(v) => Some(rt.to_number(v)?), None => None };
            let d_arg = match args.get(2) { Some(v) => Some(rt.to_number(v)?), None => None };
            if !y.is_finite() || m_arg.is_some_and(|x| !x.is_finite()) || d_arg.is_some_and(|x| !x.is_finite()) {
                return set_this_time(rt, &this, f64::NAN);
            }
            let base = if t.is_finite() { t } else { 0.0 };
            let Some(dt) = dt_from_ms(base) else { return set_this_time(rt, &this, f64::NAN); };
            let m = m_arg.map(|x| x.trunc() as i32).unwrap_or(if t.is_finite() { dt.month0() as i32 } else { 0 });
            let d = d_arg.map(|x| x.trunc() as i32).unwrap_or(if t.is_finite() { dt.day() as i32 } else { 1 });
            let new_t = make_utc_ms(y.trunc() as i32, m, d, if t.is_finite() { dt.hour() as i32 } else { 0 }, if t.is_finite() { dt.minute() as i32 } else { 0 }, if t.is_finite() { dt.second() as i32 } else { 0 }, if t.is_finite() { (dt.timestamp_subsec_millis() % 1000) as i32 } else { 0 });
            set_this_time(rt, &this, new_t)
        }),
    );

    let sym_tp = rt.symbols.to_primitive;
    let tp_fn = rt.create_native_function(
        "[Symbol.toPrimitive]",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("Date[Symbol.toPrimitive] called on non-object"));
            };
            let Some(JsValue::String(hint)) = args.first() else {
                return Err(JsError::type_err("Invalid hint for Date[Symbol.toPrimitive]"));
            };
            let order = match hint.as_str() {
                "string" | "default" => ["toString", "valueOf"],
                "number" => ["valueOf", "toString"],
                _ => return Err(JsError::type_err("Invalid hint for Date[Symbol.toPrimitive]")),
            };
            for m in order {
                let f = rt.get_property(oid, m)?;
                if rt.is_callable(&f) {
                    let res = rt.call_function(f, JsValue::Object(oid), &[])?;
                    if !res.is_object() {
                        return Ok(res);
                    }
                }
            }
            Err(JsError::type_err("Cannot convert Date to primitive"))
        }),
    );
    rt.heap.get_obj_mut(proto).set_own_prop(
        PropertyKey::Symbol(sym_tp),
        PropEntry::data(JsValue::Object(tp_fn), false, false, true),
    );
    let _ = set_symbol_method;

    let ctor = rt.create_spec_constructor(
        "Date",
        7,
        proto,
        Rc::new(|rt, _, args| {
            let is_new = matches!(&rt.native_new_target, Some(nt) if !nt.is_undefined());
            if !is_new {
                let dt = Local::now();
                return Ok(JsValue::String(
                    dt.format("%a %b %d %Y %H:%M:%S GMT%z").to_string(),
                ));
            }
            let obj_proto = rt.get_prototype_from_new_target(rt.intrinsics.date_proto)?;
            let ms = if args.is_empty() {
                now_ms()
            } else if args.len() == 1 {
                let v = &args[0];
                if let JsValue::Object(oid) = v {
                    if let ObjectKind::Date(t) = rt.heap.get_obj(*oid).kind {
                        time_clip(t)
                    } else {
                        let p = rt.to_primitive(v, "default")?;
                        match p {
                            JsValue::String(s) => parse_date_string(&s),
                            other => time_clip(rt.to_number(&other)?),
                        }
                    }
                } else {
                    match v {
                        JsValue::String(s) => parse_date_string(s),
                        other => time_clip(rt.to_number(other)?),
                    }
                }
            } else {
                let y = rt.to_number(&args[0])?;
                let m = rt.to_number(&args[1])?;
                let d = match args.get(2) {
                    Some(v) => rt.to_number(v)?,
                    None => 1.0,
                };
                let h = match args.get(3) {
                    Some(v) => rt.to_number(v)?,
                    None => 0.0,
                };
                let min = match args.get(4) {
                    Some(v) => rt.to_number(v)?,
                    None => 0.0,
                };
                let s = match args.get(5) {
                    Some(v) => rt.to_number(v)?,
                    None => 0.0,
                };
                let ms_v = match args.get(6) {
                    Some(v) => rt.to_number(v)?,
                    None => 0.0,
                };
                if !y.is_finite()
                    || !m.is_finite()
                    || !d.is_finite()
                    || !h.is_finite()
                    || !min.is_finite()
                    || !s.is_finite()
                    || !ms_v.is_finite()
                {
                    f64::NAN
                } else {
                    let yi = y.trunc() as i32;
                    let full_y = if (0..=99).contains(&yi) { 1900 + yi } else { yi };
                    make_utc_ms(
                        full_y,
                        m.trunc() as i32,
                        d.trunc() as i32,
                        h.trunc() as i32,
                        min.trunc() as i32,
                        s.trunc() as i32,
                        ms_v.trunc() as i32,
                    )
                }
            };
            let obj = rt.heap.alloc_object(JsObject::new(
                Some(obj_proto),
                ObjectKind::Date(ms),
            ));
            Ok(JsValue::Object(obj))
        }),
    );

    set_method(
        rt,
        ctor,
        "now",
        0,
        Rc::new(|_, _, _| Ok(JsValue::Number(now_ms()))),
    );
    set_method(
        rt,
        ctor,
        "parse",
        1,
        Rc::new(|rt, _, args| {
            let s = rt.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
            Ok(JsValue::Number(parse_date_string(&s)))
        }),
    );
    set_method(
        rt,
        ctor,
        "UTC",
        7,
        Rc::new(|rt, _, args| {
            if args.is_empty() {
                return Ok(JsValue::Number(f64::NAN));
            }
            let y = rt.to_number(&args[0])?;
            let m = match args.get(1) {
                Some(v) => rt.to_number(v)?,
                None => 0.0,
            };
            let d = match args.get(2) {
                Some(v) => rt.to_number(v)?,
                None => 1.0,
            };
            let h = match args.get(3) {
                Some(v) => rt.to_number(v)?,
                None => 0.0,
            };
            let min = match args.get(4) {
                Some(v) => rt.to_number(v)?,
                None => 0.0,
            };
            let s = match args.get(5) {
                Some(v) => rt.to_number(v)?,
                None => 0.0,
            };
            let ms_v = match args.get(6) {
                Some(v) => rt.to_number(v)?,
                None => 0.0,
            };
            if !y.is_finite()
                || !m.is_finite()
                || !d.is_finite()
                || !h.is_finite()
                || !min.is_finite()
                || !s.is_finite()
                || !ms_v.is_finite()
            {
                return Ok(JsValue::Number(f64::NAN));
            }
            let yi = y.trunc() as i32;
            let full_y = if (0..=99).contains(&yi) { 1900 + yi } else { yi };
            Ok(JsValue::Number(make_utc_ms(
                full_y,
                m.trunc() as i32,
                d.trunc() as i32,
                h.trunc() as i32,
                min.trunc() as i32,
                s.trunc() as i32,
                ms_v.trunc() as i32,
            )))
        }),
    );

    set_global(rt, "Date", JsValue::Object(ctor));
}
