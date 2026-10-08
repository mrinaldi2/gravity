//! The served IPA's sha is cached by a stamp a swapped file can't keep
//! (CE review of H-241, M1).

mod common;

use std::path::PathBuf;

use common::release_phone::{ours, published, set_status};
use common::WsClient;
use hermesd::board::release::model::ReleaseStatus;
use serde_json::{json, Value};

/// The served IPA swapped for other bytes of the same size, its old mtime
/// set back: the next `release_install` hashes it again and gives no link,
/// whether it was overwritten in place or renamed over (CE review of H-241,
/// M1).
#[tokio::test]
async fn a_same_size_swap_with_the_old_mtime_loses_the_link() {
    let (r, id, served, _) = published().await;
    let (page_url, _) = ours(&id);
    set_status(&r.pair.d, &id, ReleaseStatus::AwaitingOwner);
    let ipa = served.join("TheHermes.ipa");
    let mut owner = WsClient::connect(&r.pair.d).await;
    let info = json!({"type": "release_install", "release_id": id});
    let set_mtime = |path: &PathBuf, to| {
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(to)
            .unwrap()
    };
    let original = std::fs::read(&ipa).unwrap();
    let mut other = original.clone();
    other[0] ^= 0xff;
    let swapped = served.join("swap.ipa");
    let swaps: [&dyn Fn(); 2] = [&|| std::fs::write(&ipa, &other).unwrap(), &|| {
        std::fs::write(&swapped, &other).unwrap();
        std::fs::rename(&swapped, &ipa).unwrap();
    }];
    for swap in swaps {
        std::fs::write(&ipa, &original).unwrap();
        let got = owner.request(info.clone()).await;
        assert_eq!(got["install"]["page_url"], page_url.as_str(), "{got}");
        let mtime = std::fs::metadata(&ipa).unwrap().modified().unwrap();
        swap();
        set_mtime(&ipa, mtime);
        let meta = std::fs::metadata(&ipa).unwrap();
        assert_eq!(
            (meta.len(), meta.modified().unwrap()),
            (original.len() as u64, mtime)
        );
        let got = owner.request(info.clone()).await;
        assert_eq!(got["install"]["page_url"], Value::Null, "{got}");
        assert_eq!(got["install"]["install_url"], Value::Null, "{got}");
    }
}
