//! Exact Google account-host membership data, not permission by itself.
//! Used for fixed diagnostics and the separately guarded account-window-only
//! SetSID policy in youtube-regional-account-navigation-20261015.json.
//! Never use membership alone for navigation, popups, requests, IPC, account
//! completion or access to credentials; common player/popup policy is unchanged.
//!
//! Suffix data: Chromium GOOGLE_TLD_LIST, read in full on 2026-10-15:
//! https://raw.githubusercontent.com/chromium/chromium/main/components/google/core/common/google_tld_list.h
//! Host-boundary reference: components/google/core/common/google_util.cc.
//! A listed host does not prove that an authentication handoff was attempted.
//
// Chromium suffix data copyright 2016 The Chromium Authors.
// Redistribution and use in source and binary forms, with or without
// modification, are permitted provided that the following conditions are met:
// * Redistributions of source code must retain the above copyright notice,
//   this list of conditions and the following disclaimer.
// * Redistributions in binary form must reproduce the above copyright notice,
//   this list of conditions and the following disclaimer in the documentation
//   and/or other materials provided with the distribution.
// * Neither the name of Google LLC nor the names of its contributors may be used
//   to endorse or promote products derived from this software without specific
//   prior written permission.
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
// AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
// IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
// ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER OR CONTRIBUTORS BE LIABLE
// FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
// DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
// SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED
// AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
// (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
// SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

const GOOGLE_SUFFIXES: &[&str] = &[
    "ac", "ad", "ae", "af", "ag", "al", "am", "as", "at", "aw", "az", "ba",
    "be", "bf", "bg", "bi", "bj", "bm", "bn", "bo", "bs", "bt", "by", "ca",
    "cat", "cc", "cd", "cf", "cg", "ch", "ci", "cl", "cm", "cn", "co",
    "co.ao", "co.bw", "co.ck", "co.cr", "co.gy", "co.hu", "co.id", "co.il",
    "co.im", "co.in", "co.je", "co.jp", "co.ke", "co.kr", "co.ls", "co.ma",
    "co.mz", "co.nz", "co.rs", "co.th", "co.tz", "co.ug", "co.uk", "co.uz",
    "co.ve", "co.vi", "co.za", "co.zm", "co.zw", "com", "com.af", "com.ag",
    "com.ai", "com.ar", "com.au", "com.bd", "com.bh", "com.bi", "com.bn",
    "com.bo", "com.br", "com.by", "com.bz", "com.cn", "com.co", "com.cu",
    "com.cy", "com.do", "com.dz", "com.ec", "com.eg", "com.er", "com.et",
    "com.fj", "com.ge", "com.gh", "com.gi", "com.gp", "com.gr", "com.gt",
    "com.gy", "com.hk", "com.ht", "com.iq", "com.jm", "com.jo", "com.kh",
    "com.kw", "com.kz", "com.lb", "com.lv", "com.ly", "com.mm", "com.mt",
    "com.mx", "com.my", "com.na", "com.nc", "com.nf", "com.ng", "com.ni",
    "com.np", "com.nr", "com.om", "com.pa", "com.pe", "com.pg", "com.ph",
    "com.pk", "com.pl", "com.pr", "com.ps", "com.pt", "com.py", "com.qa",
    "com.ru", "com.sa", "com.sb", "com.sg", "com.sl", "com.sv", "com.tj",
    "com.tm", "com.tn", "com.tr", "com.tw", "com.ua", "com.uy", "com.vc",
    "com.ve", "com.vn", "cv", "cz", "de", "dj", "dk", "dm", "do", "dz",
    "ec", "ee", "es", "eu", "fi", "fm", "fr", "ga", "gd", "ge", "gf", "gg",
    "gl", "gm", "gp", "gr", "gw", "gy", "hk", "hn", "hr", "ht", "hu", "ie",
    "im", "in", "info", "io", "iq", "is", "it", "it.ao", "je", "jo", "jobs",
    "jp", "kg", "ki", "km", "kr", "kz", "la", "li", "lk", "lt", "lu", "lv",
    "ma", "md", "me", "mg", "mh", "mk", "ml", "mn", "mr", "ms", "mu", "mv",
    "mw", "mx", "ne", "ne.jp", "net", "ng", "nl", "no", "nr", "nu",
    "off.ai", "org", "pf", "ph", "pk", "pl", "pn", "ps", "pt", "qa", "re",
    "ro", "rs", "ru", "rw", "sc", "se", "sg", "sh", "si", "sk", "sl", "sm",
    "sn", "so", "sr", "st", "sz", "td", "tel", "tg", "tk", "tl", "tm", "tn",
    "to", "tt", "tw", "ua", "us", "uz", "vg", "vn", "vu", "ws", "yt",
];

pub(super) fn is_known_regional_account_host(host: &str) -> bool {
    host.strip_prefix("accounts.google.")
        .is_some_and(|suffix| suffix != "com" && GOOGLE_SUFFIXES.contains(&suffix))
}

#[cfg(test)]
pub(super) fn known_suffixes_for_test() -> &'static [&'static str] {
    GOOGLE_SUFFIXES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn official_suffix_data_is_unique_and_plain_without_wildcards() {
        let mut unique = std::collections::HashSet::new();
        assert!(GOOGLE_SUFFIXES.len() > 200);
        for suffix in GOOGLE_SUFFIXES {
            assert!(unique.insert(suffix));
            assert!(!suffix.is_empty());
            assert!(suffix.bytes().all(|b| b.is_ascii_lowercase() || b == b'.'));
            assert!(!suffix.starts_with('.') && !suffix.ends_with('.'));
            assert_eq!(is_known_regional_account_host(&format!("accounts.google.{suffix}")), *suffix != "com");
        }
    }

    #[test]
    fn classification_requires_the_exact_accounts_host_boundary() {
        for host in ["accounts.google.co.kr", "accounts.google.ro", "accounts.google.fr", "accounts.google.co.uk"] {
            assert!(is_known_regional_account_host(host));
        }
        for host in ["accounts.google.com", "google.co.kr", "www.google.co.kr", "evil.accounts.google.co.kr",
            "accounts.google.co.kr.evil.test", "accounts.google.com.evil.test", "accounts.google.evil.test",
            "accounts.google.invalid", "accounts.google.co.kr.", "accounts.google.co.kr:443", "accounts.google..co.kr"] {
            assert!(!is_known_regional_account_host(host), "{host}");
        }
    }
}
