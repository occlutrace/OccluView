use super::*;

    fn assert_vec3_close(actual: [f32; 3], expected: [f32; 3]) {
        for (a, e) in actual.into_iter().zip(expected) {
            assert!(
                (a - e).abs() < 1.0e-5,
                "expected {expected:?}, got {actual:?}"
            );
        }
    }

    /// Build a minimal GLB: one triangle, FLOAT VEC3 positions + UINT indices.
    fn one_triangle_glb() -> Vec<u8> {
        let json = br#"{"asset":{"version":"2.0"},
"scenes":[{"nodes":[0]}],
"nodes":[{"mesh":0}],
"meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1}]}],
"accessors":[{"bufferView":0,"count":3,"type":"VEC3","componentType":5126},
             {"bufferView":1,"count":3,"type":"SCALAR","componentType":5125}],
"bufferViews":[{"buffer":0,"byteLength":36},{"buffer":0,"byteOffset":36,"byteLength":12}],
"buffers":[{"byteLength":48}]}"#;
        let mut bin = Vec::new();
        // 3 positions: (0,0,0),(1,0,0),(0,1,0)
        for f in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
            bin.extend_from_slice(&f.to_le_bytes());
        }
        // 3 indices
        for i in 0u32..3 {
            bin.extend_from_slice(&i.to_le_bytes());
        }
        glb::build_glb(json, &bin)
    }

    #[test]
    fn reads_minimal_triangle() {
        let bytes = one_triangle_glb();
        let mesh = read(&bytes).expect("valid GLB");
        assert_eq!(mesh.triangle_count(), 1);
        assert_eq!(mesh.vertices().len(), 3);
        assert_eq!(mesh.vertices()[1].position, [1.0, 0.0, 0.0]);
    }

    #[test]
    fn rejects_non_glb() {
        assert!(read(b"not gltf").is_err());
    }

    #[test]
    fn rejects_unsupported_primitive_mode() {
        // mode 6 (triangle fan) -> rejected.
        let json = br#"{"asset":{"version":"2.0"},
"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],
"meshes":[{"primitives":[{"mode":6,"attributes":{"POSITION":0}}]}],
"accessors":[{"bufferView":0,"count":3,"type":"VEC3","componentType":5126}],
"bufferViews":[{"buffer":0,"byteLength":36}],"buffers":[{"byteLength":36}]}"#;
        let bin = [0u8; 36];
        let bytes = glb::build_glb(json, &bin);
        let err = read(&bytes).unwrap_err();
        assert!(matches!(err, FormatError::Malformed { .. }));
    }

    #[test]
    fn handles_non_indexed_primitive() {
        // No indices attribute; 3 vertices -> 1 implicit triangle.
        let json = br#"{"asset":{"version":"2.0"},
"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],
"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}],
"accessors":[{"bufferView":0,"count":3,"type":"VEC3","componentType":5126}],
"bufferViews":[{"buffer":0,"byteLength":36}],"buffers":[{"byteLength":36}]}"#;
        let bin = [0u8; 36];
        let bytes = glb::build_glb(json, &bin);
        let mesh = read(&bytes).expect("valid");
        assert_eq!(mesh.triangle_count(), 1);
    }

    /// Regression: index values must round-trip correctly. An earlier bug had
    /// `read_indices` pass `&bytes[off..]` (a tail slice, not exactly 4 bytes)
    /// to `u32_at`, whose `try_into().unwrap_or([0;4])` then silently zeroed
    /// every index — producing degenerate `(0,0,0)` triangles and empty renders
    /// for any real GLB. This test uses non-trivial index values so the bug
    /// surfaces as a value mismatch, not just a count check.
    #[test]
    fn index_values_round_trip_exactly() {
        // 6 vertices, 2 triangles with indices (1,4,2) and (5,0,3).
        let json = br#"{"asset":{"version":"2.0"},
"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],
"meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1}]}],
"accessors":[{"bufferView":0,"count":6,"type":"VEC3","componentType":5126},
             {"bufferView":1,"count":6,"type":"SCALAR","componentType":5125}],
"bufferViews":[{"buffer":0,"byteLength":72},{"buffer":0,"byteOffset":72,"byteLength":24}],
"buffers":[{"byteLength":96}]}"#;
        let mut bin = Vec::new();
        // 6 positions (values irrelevant to this test).
        for _ in 0..18 {
            bin.extend_from_slice(&0.0_f32.to_le_bytes());
        }
        // 6 u32 indices: 1,4,2,5,0,3
        for i in [1u32, 4, 2, 5, 0, 3] {
            bin.extend_from_slice(&i.to_le_bytes());
        }
        let bytes = glb::build_glb(json, &bin);
        let mesh = read(&bytes).expect("valid GLB");
        assert_eq!(mesh.indices(), &[1, 4, 2, 5, 0, 3]);
    }

    /// Regression companion: USHORT (5123) indices also round-trip exactly.
    #[test]
    fn ushort_index_values_round_trip() {
        let json = br#"{"asset":{"version":"2.0"},
"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],
"meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1}]}],
"accessors":[{"bufferView":0,"count":6,"type":"VEC3","componentType":5126},
             {"bufferView":1,"count":6,"type":"SCALAR","componentType":5123}],
"bufferViews":[{"buffer":0,"byteLength":72},{"buffer":0,"byteOffset":72,"byteLength":12}],
"buffers":[{"byteLength":84}]}"#;
        let mut bin = Vec::new();
        for _ in 0..18 {
            bin.extend_from_slice(&0.0_f32.to_le_bytes());
        }
        for i in [1u16, 4, 2, 5, 0, 3] {
            bin.extend_from_slice(&i.to_le_bytes());
        }
        let bytes = glb::build_glb(json, &bin);
        let mesh = read(&bytes).expect("valid GLB");
        assert_eq!(mesh.indices(), &[1, 4, 2, 5, 0, 3]);
    }

    /// End-to-end: a GLB with `TEXCOORD_0` + a material base-color texture
    /// (PNG embedded in a bufferView) round-trips UVs and decodes the texture.
    #[test]
    fn textured_glb_round_trips_uvs_and_texture() {
        // Encode a 2×2 red PNG as the texture image bytes.
        let png_bytes: Vec<u8> = {
            let img = image::RgbaImage::from_raw(
                2,
                2,
                vec![
                    255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255,
                ],
            )
            .expect("image dims");
            let mut buf = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgba8(img)
                .write_to(&mut buf, image::ImageFormat::Png)
                .expect("encode png");
            buf.into_inner()
        };
        let png_len = png_bytes.len();

        // BIN layout:
        //   [0..72)      positions (6 verts × 12 bytes = 72, values irrelevant)
        //   [72..120)    uvs (6 verts × 8 bytes = 48)
        //   [120..132)   indices (6 × u16 = 12)
        //   [132..132+png_len)  PNG image bytes
        let uv_start = 72usize;
        let idx_start = uv_start + 48;
        let img_start = idx_start + 12;
        let total = img_start + png_len;

        let json = format!(
            r#"{{"asset":{{"version":"2.0"}},
"scenes":[{{"nodes":[0]}}],"nodes":[{{"mesh":0}}],
"meshes":[{{"primitives":[{{"attributes":{{"POSITION":0,"TEXCOORD_0":1}},"indices":2,"material":0}}]}}],
"materials":[{{"pbrMetallicRoughness":{{"baseColorTexture":{{"index":0}}}}}}],
"textures":[{{"source":0}}],
"images":[{{"bufferView":3,"mimeType":"image/png"}}],
"accessors":[{{"bufferView":0,"count":6,"type":"VEC3","componentType":5126}},
             {{"bufferView":1,"count":6,"type":"VEC2","componentType":5126}},
             {{"bufferView":2,"count":6,"type":"SCALAR","componentType":5123}}],
"bufferViews":[{{"buffer":0,"byteLength":72}},
               {{"buffer":0,"byteOffset":{uv_start},"byteLength":48}},
               {{"buffer":0,"byteOffset":{idx_start},"byteLength":12}},
               {{"buffer":0,"byteOffset":{img_start},"byteLength":{png_len}}}],
"buffers":[{{"byteLength":{total}}}]}}"#
        );

        let mut bin = Vec::with_capacity(total);
        // 6 positions (zeros).
        bin.extend(std::iter::repeat_n(0u8, 72));
        // 6 UVs: (0,0) (1,0) (0.5,1) (0,0) (1,0) (0.5,1).
        for &(u, v) in &[
            (0.0f32, 0.0f32),
            (1.0, 0.0),
            (0.5, 1.0),
            (0.0, 0.0),
            (1.0, 0.0),
            (0.5, 1.0),
        ] {
            bin.extend_from_slice(&u.to_le_bytes());
            bin.extend_from_slice(&v.to_le_bytes());
        }
        // 6 u16 indices: 0,1,2,3,4,5.
        for i in [0u16, 1, 2, 3, 4, 5] {
            bin.extend_from_slice(&i.to_le_bytes());
        }
        // PNG bytes.
        bin.extend_from_slice(&png_bytes);
        assert_eq!(bin.len(), total);

        let bytes = glb::build_glb(json.as_bytes(), &bin);
        let mesh = read(&bytes).expect("valid textured GLB");

        // UVs round-tripped.
        assert!(mesh.has_uvs());
        let verts = mesh.vertices();
        assert_eq!(verts.len(), 6);
        assert_eq!(verts[0].uv, [0.0, 0.0]);
        assert_eq!(verts[1].uv, [1.0, 0.0]);
        assert_eq!(verts[2].uv, [0.5, 1.0]);

        // Texture decoded + attached.
        let tex = mesh.texture().expect("texture should be attached");
        assert_eq!(tex.width, 2);
        assert_eq!(tex.height, 2);
        // Every pixel is red.
        assert!(tex.rgba.chunks_exact(4).all(|p| p == [255, 0, 0, 255]));
    }

    /// A GLB with no texture (plain geometry) must not attach a texture.
    #[test]
    fn untextured_glb_has_no_texture() {
        let bytes = one_triangle_glb();
        let mesh = read(&bytes).expect("valid GLB");
        assert!(mesh.texture().is_none(), "untextured mesh got a texture");
        assert!(!mesh.has_uvs(), "mesh with no TEXCOORD_0 reported has_uvs");
    }

    #[test]
    fn applies_node_matrix_transform_through_hierarchy() {
        let json = br#"{"asset":{"version":"2.0"},
"scenes":[{"nodes":[0]}],
"nodes":[
  {"children":[1],"translation":[10,0,0]},
  {"mesh":0,"matrix":[1,0,0,0,0,1,0,0,0,0,1,0,0,5,0,1]}
],
"meshes":[{"primitives":[{"attributes":{"POSITION":0,"NORMAL":1},"indices":2}]}],
"accessors":[
  {"bufferView":0,"count":3,"type":"VEC3","componentType":5126},
  {"bufferView":1,"count":3,"type":"VEC3","componentType":5126},
  {"bufferView":2,"count":3,"type":"SCALAR","componentType":5125}
],
"bufferViews":[
  {"buffer":0,"byteLength":36},
  {"buffer":0,"byteOffset":36,"byteLength":36},
  {"buffer":0,"byteOffset":72,"byteLength":12}
],
"buffers":[{"byteLength":84}]}"#;
        let mut bin = Vec::new();
        for f in [1.0f32, 2.0, 3.0, 2.0, 2.0, 3.0, 1.0, 3.0, 3.0] {
            bin.extend_from_slice(&f.to_le_bytes());
        }
        for f in [0.0f32, 0.0, 2.0, 0.0, 0.0, 2.0, 0.0, 0.0, 2.0] {
            bin.extend_from_slice(&f.to_le_bytes());
        }
        for i in 0u32..3 {
            bin.extend_from_slice(&i.to_le_bytes());
        }

        let mesh = read(&glb::build_glb(json, &bin)).expect("valid GLB");
        assert_eq!(mesh.triangle_count(), 1);
        assert_vec3_close(mesh.vertices()[0].position, [11.0, 7.0, 3.0]);
        assert_vec3_close(mesh.vertices()[0].normal, [0.0, 0.0, 1.0]);
    }

    #[test]
    fn applies_node_trs_transform_and_normalizes_normals() {
        let json = br#"{"asset":{"version":"2.0"},
"scenes":[{"nodes":[0]}],
"nodes":[{"mesh":0,"translation":[10,20,30],"rotation":[0,0,0.70710677,0.70710677],"scale":[2,3,4]}],
"meshes":[{"primitives":[{"attributes":{"POSITION":0,"NORMAL":1},"indices":2}]}],
"accessors":[
  {"bufferView":0,"count":3,"type":"VEC3","componentType":5126},
  {"bufferView":1,"count":3,"type":"VEC3","componentType":5126},
  {"bufferView":2,"count":3,"type":"SCALAR","componentType":5125}
],
"bufferViews":[
  {"buffer":0,"byteLength":36},
  {"buffer":0,"byteOffset":36,"byteLength":36},
  {"buffer":0,"byteOffset":72,"byteLength":12}
],
"buffers":[{"byteLength":84}]}"#;
        let mut bin = Vec::new();
        for f in [1.0f32, 0.0, 0.0, 0.0, 1.0, 0.0, -1.0, 0.0, 0.0] {
            bin.extend_from_slice(&f.to_le_bytes());
        }
        for f in [1.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0] {
            bin.extend_from_slice(&f.to_le_bytes());
        }
        for i in 0u32..3 {
            bin.extend_from_slice(&i.to_le_bytes());
        }

        let mesh = read(&glb::build_glb(json, &bin)).expect("valid GLB");
        assert_eq!(mesh.triangle_count(), 1);
        assert_vec3_close(mesh.vertices()[0].position, [10.0, 22.0, 30.0]);
        assert_vec3_close(mesh.vertices()[0].normal, [0.0, 1.0, 0.0]);
    }

    #[test]
    fn transforms_normals_with_inverse_transpose_for_non_uniform_scale() {
        let json = br#"{"asset":{"version":"2.0"},
"scenes":[{"nodes":[0]}],
"nodes":[{"mesh":0,"scale":[2,1,1]}],
"meshes":[{"primitives":[{"attributes":{"POSITION":0,"NORMAL":1},"indices":2}]}],
"accessors":[
  {"bufferView":0,"count":3,"type":"VEC3","componentType":5126},
  {"bufferView":1,"count":3,"type":"VEC3","componentType":5126},
  {"bufferView":2,"count":3,"type":"SCALAR","componentType":5125}
],
"bufferViews":[
  {"buffer":0,"byteLength":36},
  {"buffer":0,"byteOffset":36,"byteLength":36},
  {"buffer":0,"byteOffset":72,"byteLength":12}
],
"buffers":[{"byteLength":84}]}"#;
        let mut bin = Vec::new();
        for f in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
            bin.extend_from_slice(&f.to_le_bytes());
        }
        for f in [
            0.707_106_77_f32,
            0.707_106_77,
            0.0,
            0.707_106_77,
            0.707_106_77,
            0.0,
            0.707_106_77,
            0.707_106_77,
            0.0,
        ] {
            bin.extend_from_slice(&f.to_le_bytes());
        }
        for i in 0u32..3 {
            bin.extend_from_slice(&i.to_le_bytes());
        }

        let mesh = read(&glb::build_glb(json, &bin)).expect("valid GLB");
        assert_vec3_close(mesh.vertices()[0].normal, [0.447_213_6, 0.894_427_2, 0.0]);
    }

    #[test]
    fn tolerates_material_without_pbr_base_color_texture() {
        let json = br#"{"asset":{"version":"2.0"},
"scenes":[{"nodes":[0]}],
"nodes":[{"mesh":0}],
"meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1,"material":0}]}],
"materials":[{"name":"plain-material"}],
"accessors":[
  {"bufferView":0,"count":3,"type":"VEC3","componentType":5126},
  {"bufferView":1,"count":3,"type":"SCALAR","componentType":5125}
],
"bufferViews":[
  {"buffer":0,"byteLength":36},
  {"buffer":0,"byteOffset":36,"byteLength":12}
],
"buffers":[{"byteLength":48}]}"#;
        let mut bin = Vec::new();
        for f in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
            bin.extend_from_slice(&f.to_le_bytes());
        }
        for i in 0u32..3 {
            bin.extend_from_slice(&i.to_le_bytes());
        }

        let mesh = read(&glb::build_glb(json, &bin)).expect("valid GLB");
        assert_eq!(mesh.triangle_count(), 1);
        assert!(mesh.texture().is_none());
    }

    #[test]
    fn rejects_index_count_not_divisible_by_three() {
        let json = br#"{"asset":{"version":"2.0"},
"scenes":[{"nodes":[0]}],
"nodes":[{"mesh":0}],
"meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1}]}],
"accessors":[
  {"bufferView":0,"count":3,"type":"VEC3","componentType":5126},
  {"bufferView":1,"count":4,"type":"SCALAR","componentType":5125}
],
"bufferViews":[
  {"buffer":0,"byteLength":36},
  {"buffer":0,"byteOffset":36,"byteLength":16}
],
"buffers":[{"byteLength":52}]}"#;
        let mut bin = Vec::new();
        for f in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
            bin.extend_from_slice(&f.to_le_bytes());
        }
        for i in [0u32, 1, 2, 0] {
            bin.extend_from_slice(&i.to_le_bytes());
        }

        let err = read(&glb::build_glb(json, &bin)).unwrap_err();
        assert!(matches!(err, FormatError::Malformed { .. }));
    }
