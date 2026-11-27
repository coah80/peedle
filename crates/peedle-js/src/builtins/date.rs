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

    let sym_tp = rt.symbols.to_primitive;
    set_symbol_method(
        rt,
        proto,
        sym_tp,
        "[Symbol.toPrimitive]",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("Date[Symbol.toPrimitive] called on non-object"));
            };
            let hint = rt.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
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

    let ctor = rt.create_native_constructor(
        "Date",
        7,
        proto,
        Rc::new(|rt, this, args| {
            let is_new = matches!(
                this,
                JsValue::Object(oid) if rt.heap.get_obj(oid).prototype == Some(rt.intrinsics.date_proto)
            );
            if !is_new {
                let dt = Local::now();
                return Ok(JsValue::String(
                    dt.format("%a %b %d %Y %H:%M:%S GMT%z").to_string(),
                ));
            }
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
                Some(rt.intrinsics.date_proto),
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
