use peedle_url::Url;

const BLOCKED_DOMAINS: &[&str] = &[
    "doubleclick.net",
    "googlesyndication.com",
    "googleadservices.com",
    "googletagservices.com",
    "googletagmanager.com",
    "google-analytics.com",
    "pagead2.googlesyndication.com",
    "adservice.google.com",
    "amazon-adsystem.com",
    "adnxs.com",
    "criteo.com",
    "criteo.net",
    "taboola.com",
    "outbrain.com",
    "pubmatic.com",
    "rubiconproject.com",
    "openx.net",
    "casalemedia.com",
    "scorecardresearch.com",
    "quantserve.com",
    "quantcount.com",
    "connect.facebook.net",
    "analytics.tiktok.com",
    "hotjar.com",
    "segment.io",
    "segment.com",
    "mixpanel.com",
    "amplitude.com",
    "permutive.com",
    "chartbeat.com",
    "chartbeat.net",
    "parsely.com",
    "teads.tv",
    "yieldmo.com",
    "sharethrough.com",
    "3lift.com",
    "media.net",
    "moatads.com",
    "adsafeprotected.com",
    "doubleverify.com",
    "imrworldwide.com",
    "krxd.net",
    "bluekai.com",
    "demdex.net",
    "omtrdc.net",
    "2o7.net",
    "cookielaw.org",
    "onetrust.com",
    "cookiebot.com",
    "trustarc.com",
    "privacymanager.io",
    "cdn.ampproject.org",
    "adsystem.com",
    "ads-twitter.com",
    "bat.bing.com",
    "clarity.ms",
    "fullstory.com",
    "bugsnag.com",
    "newrelic.com",
    "nr-data.net",
    "branch.io",
    "appsflyer.com",
    "adjust.com",
    "onesignal.com",
    "pushengage.com",
    "revcontent.com",
    "mgid.com",
    "zedo.com",
    "adroll.com",
    "bidswitch.net",
    "rlcdn.com",
    "liadm.com",
    "id5-sync.com",
    "crwdcntrl.net",
    "exelator.com",
    "tapad.com",
    "agkn.com",
    "narrative.io",
    "dotomi.com",
    "smartadserver.com",
    "adform.net",
    "zemanta.com",
    "indexww.com",
    "contextweb.com",
    "lijit.com",
    "sonobi.com",
    "spotxchange.com",
    "gumgum.com",
    "kargo.com",
    "nativo.com",
    "dianomi.com",
    "connatix.com",
    "primis.tech",
    "playwire.com",
    "mediavine.com",
    "adthrive.com",
    "cafemedia.com",
    "ezoic.net",
    "freestar.com",
];

const BLOCKED_PATH_TOKENS: &[&str] = &[
    "/pagead/",
    "/gampad/",
    "/pcs/click",
    "/gtag/js",
    "/gtm.js",
    "/fbevents.js",
    "/ads/prebid",
    "/prebid.js",
    "/gpt.js",
    "/pubads_impl",
];

pub fn should_block_url(url: &Url) -> bool {
    let scheme = url.scheme.as_str();
    if scheme != "http" && scheme != "https" {
        return false;
    }
    let host = url.host_str().to_ascii_lowercase();
    if host.is_empty() {
        return false;
    }
    for &dom in BLOCKED_DOMAINS {
        if host == dom
            || (host.len() > dom.len()
                && host.ends_with(dom)
                && host.as_bytes()[host.len() - dom.len() - 1] == b'.')
        {
            return true;
        }
    }
    if host.starts_with("ads.")
        || host.starts_with("adserver.")
        || host.starts_with("adservice.")
        || host.starts_with("telemetry.")
        || host.starts_with("metrics.")
        || host.starts_with("pixel.")
    {
        return true;
    }
    let path = url.pathname().to_ascii_lowercase();
    for &tok in BLOCKED_PATH_TOKENS {
        if path.contains(tok) {
            return true;
        }
    }
    false
}
