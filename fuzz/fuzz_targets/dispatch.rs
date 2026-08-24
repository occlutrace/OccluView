#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = occluview_formats::dispatch::dispatch_by_extension("stl", data);
    let _ = occluview_formats::dispatch::dispatch_by_extension("ply", data);
    let _ = occluview_formats::dispatch::dispatch_by_extension("obj", data);
    let _ = occluview_formats::dispatch::dispatch_by_extension("hps", data);
    let _ = occluview_formats::dispatch::dispatch_by_extension("glb", data);
    let _ = occluview_formats::probe::probe(None, data);
    for ext in ["stl", "ply", "obj", "hps", "glb", "xyz"] {
        let _ = occluview_formats::probe::probe(Some(ext), data);
    }
});
