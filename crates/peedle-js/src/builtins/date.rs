use crate::builtins::{set_global, set_method, set_symbol_method};
use crate::error::JsError;
use crate::heap::*;
use crate::value::*;
use crate::vm::JsRuntime;
use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveDateTime, TimeZone, Timelike, Utc};
use std::rc::Rc;

const MAX_TIME: f64 = 8.64e15;
const MS_PER_DAY: f64 = 86_400_000.0;

fn time_clip(t: f64) -> f64 {
    if !t.is_finite() || t.abs() > MAX_TIME {
        f64::NAN
    } else {
        let v = t.trunc();
        if v == 0.0 { 0.0 } else { v }
    }
}

fn day_from_year(y: f64) -> f64 {
    365.0 * (y - 1970.0)
        + ((y - 1969.0) / 4.0).floor()
        - ((y - 1901.0) / 100.0).floor()
        + ((y - 1601.0) / 400.0).floor()
}

fn in_leap_year(y: f64) -> bool {
    let yi = y as i64;
    (yi % 4 == 0 && yi % 100 != 0) || (yi % 400 == 0)
}

fn day_within_year_for_month(y: f64, m: usize) -> f64 {
    const NORM: [f64; 12] = [0.0, 31.0, 59.0, 90.0, 120.0, 151.0, 181.0, 212.0, 243.0, 273.0, 304.0, 334.0];
    const LEAP: [f64; 12] = [0.0, 31.0, 60.0, 91.0, 121.0, 152.0, 182.0, 213.0, 244.0, 274.0, 305.0, 335.0];
    if in_leap_year(y) { LEAP[m] } else { NORM[m] }
}

fn make_day_f64(year: f64, month: f64, date: f64) -> f64 {
    if !year.is_finite() || !month.is_finite() || !date.is_finite() {
        return f64::NAN;
    }
    let y = year.trunc();
    let m = month.trunc();
    let dt = date.trunc();
    let ym = y + (m / 12.0).floor();
    if !ym.is_finite() {
        return f64::NAN;
    }
    let mn = m.rem_euclid(12.0) as usize;
    let day = day_from_year(ym) + day_within_year_for_month(ym, mn);
    day + dt - 1.0
}

fn make_time_f64(hour: f64, min: f64, sec: f64, ms: f64) -> f64 {
    if !hour.is_finite() || !min.is_finite() || !sec.is_finite() || !ms.is_finite() {
        return f64::NAN;
    }
    let h = hour.trunc();
    let m = min.trunc();
    let s = sec.trunc();
    let milli = ms.trunc();
    ((h * 3_600_000.0 + m * 60_000.0) + s * 1_000.0) + milli
}

fn make_date_f64(day: f64, time: f64) -> f64 {
    if !day.is_finite() || !time.is_finite() {
        return f64::NAN;
    }
    let tv = day * MS_PER_DAY + time;
    if !tv.is_finite() { f64::NAN } else { tv }
}

fn decompose_utc_ms(ms: f64) -> Option<(i32, u32, u32, u32, u32, u32, u32)> {
    if !ms.is_finite() || ms.abs() > MAX_TIME {
        return None;
    }
    let t = ms.trunc() as i64;
    let day = t.div_euclid(86_400_000);
    let rem = t.rem_euclid(86_400_000) as u32;
    let hour = rem / 3_600_000;
    let min = (rem % 3_600_000) / 60_000;
    let sec = (rem % 60_000) / 1_000;
    let milli = rem % 1_000;
    let z = day + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    if m <= 2 {
        y += 1;
    }
    Some((y as i32, m as u32, d as u32, hour, min, sec, milli))
}

fn format_iso_utc(ms: f64) -> Option<String> {
    let (y, m, d, h, min, s, milli) = decompose_utc_ms(ms)?;
    let y_str = if y < 0 {
        format!("-{:06}", -y)
    } else if y > 9999 {
        format!("+{:06}", y)
    } else {
        format!("{:04}", y)
    };
    Some(format!(
        "{}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        y_str, m, d, h, min, s, milli
    ))
}

fn parse_es_iso_date(s: &str) -> Option<f64> {
    let bytes = s.as_bytes();
    let mut i: usize;
    let (year, neg_zero_year) = if bytes.first() == Some(&b'+') || bytes.first() == Some(&b'-') {
        let neg = bytes[0] == b'-';
        if bytes.len() < 7 || !bytes[1..7].iter().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let yv: i64 = s[1..7].parse().ok()?;
        i = 7;
        if neg && yv == 0 {
            (0i64, true)
        } else {
            (if neg { -yv } else { yv }, false)
        }
    } else {
        if bytes.len() < 4 || !bytes[0..4].iter().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let yv: i64 = s[0..4].parse().ok()?;
        i = 4;
        (yv, false)
    };
    if neg_zero_year {
        return Some(f64::NAN);
    }
    let mut month: i64 = 1;
    let mut day: i64 = 1;
    if i < bytes.len() && bytes[i] == b'-' {
        if i + 3 > bytes.len() || !bytes[i + 1..i + 3].iter().all(|b| b.is_ascii_digit()) {
            return None;
        }
        month = s[i + 1..i + 3].parse().ok()?;
        if !(1..=12).contains(&month) {
            return Some(f64::NAN);
        }
        i += 3;
        if i < bytes.len() && bytes[i] == b'-' {
            if i + 3 > bytes.len() || !bytes[i + 1..i + 3].iter().all(|b| b.is_ascii_digit()) {
                return None;
            }
            day = s[i + 1..i + 3].parse().ok()?;
            if !(1..=31).contains(&day) {
                return Some(f64::NAN);
            }
            i += 3;
        }
    }
    if i == bytes.len() {
        let d = make_day_f64(year as f64, (month - 1) as f64, day as f64);
        return Some(time_clip(make_date_f64(d, 0.0)));
    }
    if bytes[i] != b'T' {
        return None;
    }
    i += 1;
    if i + 5 > bytes.len()
        || !bytes[i..i + 2].iter().all(|b| b.is_ascii_digit())
        || bytes[i + 2] != b':'
        || !bytes[i + 3..i + 5].iter().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let hour: i64 = s[i..i + 2].parse().ok()?;
    let min: i64 = s[i + 3..i + 5].parse().ok()?;
    i += 5;
    let mut sec: i64 = 0;
    let mut milli: i64 = 0;
    if i < bytes.len() && bytes[i] == b':' {
        if i + 3 > bytes.len() || !bytes[i + 1..i + 3].iter().all(|b| b.is_ascii_digit()) {
            return None;
        }
        sec = s[i + 1..i + 3].parse().ok()?;
        i += 3;
        if i < bytes.len() && bytes[i] == b'.' {
            i += 1;
            let frac_start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i == frac_start {
                return None;
            }
            let frac = &s[frac_start..i];
            let mut ms_str = String::new();
            for ch in frac.chars().take(3) {
                ms_str.push(ch);
            }
            while ms_str.len() < 3 {
                ms_str.push('0');
            }
            milli = ms_str.parse().ok()?;
        }
    }
    if hour > 24 || min > 59 || sec > 59 || (hour == 24 && (min != 0 || sec != 0 || milli != 0)) {
        return Some(f64::NAN);
    }
    let tz_offset_ms: Option<f64> = if i == bytes.len() {
        None
    } else if bytes[i] == b'Z' && i + 1 == bytes.len() {
        Some(0.0)
    } else if (bytes[i] == b'+' || bytes[i] == b'-') && i + 6 == bytes.len() && bytes[i + 3] == b':'
        && bytes[i + 1..i + 3].iter().all(|b| b.is_ascii_digit())
        && bytes[i + 4..i + 6].iter().all(|b| b.is_ascii_digit())
    {
        let sign = if bytes[i] == b'-' { -1.0 } else { 1.0 };
        let th: f64 = s[i + 1..i + 3].parse().ok()?;
        let tm: f64 = s[i + 4..i + 6].parse().ok()?;
        if th > 23.0 || tm > 59.0 {
            return Some(f64::NAN);
        }
        Some(sign * (th * 3_600_000.0 + tm * 60_000.0))
    } else {
        return None;
    };
    let d = make_day_f64(year as f64, (month - 1) as f64, day as f64);
    let tm = make_time_f64(hour as f64, min as f64, sec as f64, milli as f64);
    let utc = make_date_f64(d, tm);
    if let Some(off) = tz_offset_ms {
        Some(time_clip(utc - off))
    } else {
        if let Some(dt) = dt_from_ms(utc) {
            let off = Local.offset_from_utc_datetime(&dt.naive_utc());
            Some(time_clip(utc - (off.local_minus_utc() as f64) * 1000.0))
        } else {
            Some(time_clip(utc))
        }
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
    let day = make_day_f64(y as f64, m as f64, d as f64);
    let time = make_time_f64(h as f64, min as f64, s as f64, ms as f64);
    time_clip(make_date_f64(day, time))
}

fn parse_date_string(s: &str) -> f64 {
    let trimmed = s.trim();
    if let Some(iso_ms) = parse_es_iso_date(trimmed) {
        return iso_ms;
    }
    if let Ok(dt) = DateTime::parse_from_str(trimmed, "%a %b %d %Y %H:%M:%S GMT%z") {
        return time_clip(dt.timestamp_millis() as f64);
    }
    if let Ok(ndt) = NaiveDateTime::parse_from_str(trimmed, "%a, %d %b %Y %H:%M:%S GMT") {
        return time_clip(ndt.and_utc().timestamp_millis() as f64);
    }
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
                .map(|d| {
                    let v = -(d.offset().local_minus_utc() as f64) / 60.0;
                    if v == 0.0 { 0.0 } else { v }
                })
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
            let Some(s) = format_iso_utc(ms) else {
                return Err(JsError::range("Invalid time value"));
            };
            Ok(JsValue::String(s))
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
            if !t.is_finite() {
                return Ok(JsValue::Number(f64::NAN));
            }
            if !ms.is_finite() {
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
            if !t.is_finite() {
                return Ok(JsValue::Number(f64::NAN));
            }
            if !ms.is_finite() {
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
            if !t.is_finite() {
                return Ok(JsValue::Number(f64::NAN));
            }
            if !sec.is_finite() || ms_arg.is_some_and(|m| !m.is_finite()) {
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
            if !t.is_finite() {
                return Ok(JsValue::Number(f64::NAN));
            }
            if !sec.is_finite() || ms_arg.is_some_and(|m| !m.is_finite()) {
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
            if !t.is_finite() {
                return Ok(JsValue::Number(f64::NAN));
            }
            if !min.is_finite() || sec_arg.is_some_and(|x| !x.is_finite()) || ms_arg.is_some_and(|x| !x.is_finite()) {
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
            if !t.is_finite() {
                return Ok(JsValue::Number(f64::NAN));
            }
            if !min.is_finite() || sec_arg.is_some_and(|x| !x.is_finite()) || ms_arg.is_some_and(|x| !x.is_finite()) {
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
            if !t.is_finite() {
                return Ok(JsValue::Number(f64::NAN));
            }
            if !hr.is_finite() || min_arg.is_some_and(|x| !x.is_finite()) || sec_arg.is_some_and(|x| !x.is_finite()) || ms_arg.is_some_and(|x| !x.is_finite()) {
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
            if !t.is_finite() {
                return Ok(JsValue::Number(f64::NAN));
            }
            if !hr.is_finite() || min_arg.is_some_and(|x| !x.is_finite()) || sec_arg.is_some_and(|x| !x.is_finite()) || ms_arg.is_some_and(|x| !x.is_finite()) {
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
            if !t.is_finite() { return Ok(JsValue::Number(f64::NAN)); }
            if !d.is_finite() { return set_this_time(rt, &this, f64::NAN); }
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
            if !t.is_finite() { return Ok(JsValue::Number(f64::NAN)); }
            if !d.is_finite() { return set_this_time(rt, &this, f64::NAN); }
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
            if !t.is_finite() { return Ok(JsValue::Number(f64::NAN)); }
            if !m.is_finite() || d_arg.is_some_and(|x| !x.is_finite()) { return set_this_time(rt, &this, f64::NAN); }
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
            if !t.is_finite() { return Ok(JsValue::Number(f64::NAN)); }
            if !m.is_finite() || d_arg.is_some_and(|x| !x.is_finite()) { return set_this_time(rt, &this, f64::NAN); }
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
            let y_trunc = y.trunc();
            let full_y = if y_trunc >= 0.0 && y_trunc <= 99.0 {
                1900.0 + y_trunc
            } else {
                y_trunc
            };
            let day = make_day_f64(full_y, m, d);
            let time = make_time_f64(h, min, s, ms_v);
            Ok(JsValue::Number(time_clip(make_date_f64(day, time))))
        }),
    );

    set_global(rt, "Date", JsValue::Object(ctor));
}
