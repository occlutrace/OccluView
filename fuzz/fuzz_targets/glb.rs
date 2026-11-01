#![no_main]
//! Separated from `dispatch` on purpose.
//!
//! GLB is the one supported format whose payload carries an offset table the
//! file itself chooses: `bufferView` byteOffset/byteLength/byteStride and
//! `accessor` componentType/count, all of which have to agree with a chunk
//! whose length is also declared in the file. Reaching that arithmetic at all
//! requires the `glTF` magic, a parseable JSON chunk and a plausible node
//! graph, so mutations that share a budget with eleven other readers almost
//! never get there.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = occluview_formats::gltf::read(data);
});
