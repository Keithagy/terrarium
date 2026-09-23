//! Native window screenshot via `WKWebView.takeSnapshot`, which captures the
//! diagram and the panels together and needs no screen-recording permission.

use anyhow::{Context, anyhow};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager};
use tokio::sync::oneshot;

pub async fn capture_png(app: &AppHandle) -> anyhow::Result<Vec<u8>> {
    let window = app.get_webview_window("main").context("no main window")?;
    let (tx, rx) = oneshot::channel::<anyhow::Result<Vec<u8>>>();
    let tx = Arc::new(Mutex::new(Some(tx)));
    window
        .with_webview(move |webview| {
            use block2::RcBlock;
            use objc2::rc::Retained;
            use objc2::runtime::AnyObject;
            use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage};
            use objc2_foundation::{NSDictionary, NSError};
            use objc2_web_kit::{WKSnapshotConfiguration, WKWebView};

            let ptr = webview.inner() as *mut AnyObject as *mut WKWebView;
            // SAFETY: `inner()` is the WKWebView backing this window and we are on the main thread.
            let wk: &WKWebView = unsafe { &*ptr };
            let mtm = objc2::MainThreadMarker::new().expect("with_webview runs on the main thread");
            let config = unsafe { WKSnapshotConfiguration::new(mtm) };
            let tx2 = tx.clone();
            let block = RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
                let result = (|| -> anyhow::Result<Vec<u8>> {
                    if image.is_null() {
                        let msg = if error.is_null() {
                            "snapshot returned no image".to_string()
                        } else {
                            unsafe { (*error).localizedDescription().to_string() }
                        };
                        return Err(anyhow!(msg));
                    }
                    let image: &NSImage = unsafe { &*image };
                    let tiff = image
                        .TIFFRepresentation()
                        .context("no TIFF representation")?;
                    let rep: Retained<NSBitmapImageRep> =
                        NSBitmapImageRep::imageRepWithData(&tiff).context("no bitmap rep")?;
                    let props = NSDictionary::new();
                    let png = unsafe {
                        rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &props)
                    }
                    .context("png encode failed")?;
                    Ok(png.to_vec())
                })();
                if let Some(tx) = tx2.lock().unwrap().take() {
                    let _ = tx.send(result);
                }
            });
            unsafe { wk.takeSnapshotWithConfiguration_completionHandler(Some(&config), &block) };
        })
        .map_err(|e| anyhow!("with_webview: {e}"))?;
    tokio::time::timeout(std::time::Duration::from_secs(10), rx)
        .await
        .context("snapshot timed out")?
        .context("snapshot channel closed")?
}
