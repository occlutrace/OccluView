use std::collections::VecDeque;

use super::error::HpsError;

#[derive(Copy, Clone)]
struct Edge {
    start: u32,
    end: u32,
}

pub(super) fn parse(
    face_bytes: &[u8],
    face_count: usize,
    vertex_count: usize,
) -> Result<Vec<u32>, HpsError> {
    parse_mode(face_bytes, face_count, vertex_count, false)
        .or_else(|_| parse_mode(face_bytes, face_count, vertex_count, true))
}

fn parse_mode(
    face_bytes: &[u8],
    face_count: usize,
    vertex_count: usize,
    index32: bool,
) -> Result<Vec<u32>, HpsError> {
    let mut decoder = FaceDecoder::new(vertex_count, index32);
    let faces = decoder.decode(face_bytes)?;
    if faces.len() != face_count.saturating_mul(3) {
        return Err(super::malformed("face_count does not match decoded facets"));
    }
    Ok(faces)
}

struct CommandReader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> CommandReader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn eof(&self) -> bool {
        self.offset >= self.bytes.len()
    }

    fn read_u8(&mut self) -> Result<u8, HpsError> {
        let value = *self
            .bytes
            .get(self.offset)
            .ok_or_else(|| super::malformed("facet command stream is truncated"))?;
        self.offset += 1;
        Ok(value)
    }

    fn read_u16(&mut self) -> Result<u32, HpsError> {
        let bytes = self.read_array::<2>("16-bit face index is truncated")?;
        Ok(u32::from(u16::from_le_bytes(bytes)))
    }

    fn read_u32(&mut self) -> Result<u32, HpsError> {
        let bytes = self.read_array::<4>("32-bit face index is truncated")?;
        Ok(u32::from_le_bytes(bytes))
    }

    fn read_array<const N: usize>(&mut self, reason: &str) -> Result<[u8; N], HpsError> {
        let end = self
            .offset
            .checked_add(N)
            .ok_or_else(|| super::malformed(reason))?;
        let slice = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| super::malformed(reason))?;
        self.offset = end;
        slice.try_into().map_err(|_| super::malformed(reason))
    }
}

struct FaceDecoder {
    vertex_count: usize,
    index32: bool,
    global_vertex_ptr: u32,
    // The front edge is the current edge; deque order stays cyclic.
    edges: VecDeque<Edge>,
    faces: Vec<u32>,
}

impl FaceDecoder {
    fn new(vertex_count: usize, index32: bool) -> Self {
        Self {
            vertex_count,
            index32,
            global_vertex_ptr: 0,
            edges: VecDeque::new(),
            faces: Vec::new(),
        }
    }

    fn decode(&mut self, bytes: &[u8]) -> Result<Vec<u32>, HpsError> {
        let mut reader = CommandReader::new(bytes);
        while !reader.eof() {
            let command = reader.read_u8()?;
            if (command >> 4) != 0 {
                return Err(super::malformed("face command high bits must be zero"));
            }
            self.process_command(command & 0x0f, &mut reader)?;
        }
        Ok(std::mem::take(&mut self.faces))
    }

    fn next_global_vertex(&mut self) -> Result<u32, HpsError> {
        let vertex = self.global_vertex_ptr;
        self.global_vertex_ptr = self
            .global_vertex_ptr
            .checked_add(1)
            .ok_or_else(|| super::malformed("face vertex pointer overflow"))?;
        self.validate(vertex)?;
        Ok(vertex)
    }

    fn validate(&self, vertex: u32) -> Result<(), HpsError> {
        if (vertex as usize) >= self.vertex_count {
            return Err(super::malformed("face vertex index is out of bounds"));
        }
        Ok(())
    }

    fn read_index_payload(&self, reader: &mut CommandReader<'_>) -> Result<u32, HpsError> {
        if self.index32 {
            reader.read_u32()
        } else {
            reader.read_u16()
        }
    }

    fn add_face(&mut self, a: u32, b: u32, c: u32) -> Result<(), HpsError> {
        self.validate(a)?;
        self.validate(b)?;
        self.validate(c)?;
        self.faces.extend_from_slice(&[a, b, c]);
        Ok(())
    }

    fn create_restart_face(&mut self, v0: u32, v1: u32, v2: u32) -> Result<(), HpsError> {
        self.add_face(v0, v1, v2)?;
        self.edges.clear();
        self.edges.push_back(Edge { start: v0, end: v1 });
        self.edges.push_back(Edge { start: v1, end: v2 });
        self.edges.push_back(Edge { start: v2, end: v0 });
        Ok(())
    }

    fn extend_current_edge(&mut self, vertex: u32) -> Result<(), HpsError> {
        self.validate(vertex)?;
        let current = self.edges.front().copied().ok_or_else(|| {
            super::malformed("face stream cannot extend an empty edge list")
        })?;
        self.add_face(vertex, current.end, current.start)?;
        let _ = self.edges.pop_front();
        self.edges.push_front(Edge {
            start: vertex,
            end: current.end,
        });
        self.edges.push_front(Edge {
            start: current.start,
            end: vertex,
        });
        Ok(())
    }

    fn increase_edge_pointer(&mut self, n: usize) -> Result<(), HpsError> {
        if self.edges.is_empty() {
            return Err(super::malformed(
                "face stream cannot advance an empty edge list",
            ));
        }
        let edge_count = self.edges.len();
        // Opcodes advance by one or two edges, keeping each rotation constant-sized.
        self.edges.rotate_left(n % edge_count);
        Ok(())
    }

    fn handle_previous(&mut self) -> Result<(), HpsError> {
        if self.edges.len() < 2 {
            return Err(super::malformed(
                "Previous command needs at least two edges",
            ));
        }
        let (Some(prev), Some(curr)) = (self.edges.back().copied(), self.edges.front().copied())
        else {
            return Err(super::malformed(
                "Previous command needs at least two edges",
            ));
        };
        self.add_face(curr.start, prev.start, curr.end)?;

        let _ = self.edges.pop_back();
        let _ = self.edges.pop_front();
        self.edges.push_front(Edge {
            start: prev.start,
            end: curr.end,
        });
        self.edges.rotate_left(1);
        Ok(())
    }

    fn handle_next(&mut self) -> Result<(), HpsError> {
        if self.edges.len() < 2 {
            return Err(super::malformed("Next command needs at least two edges"));
        }
        let (Some(curr), Some(next)) = (self.edges.front().copied(), self.edges.get(1).copied())
        else {
            return Err(super::malformed("Next command needs at least two edges"));
        };
        self.add_face(curr.start, next.end, curr.end)?;

        let _ = self.edges.pop_front();
        let _ = self.edges.pop_front();
        self.edges.push_front(Edge {
            start: curr.start,
            end: next.end,
        });
        self.edges.rotate_left(1);
        Ok(())
    }

    fn remove_current_edge(&mut self) -> Result<(), HpsError> {
        if self.edges.is_empty() {
            return Err(super::malformed(
                "Remove command needs a non-empty edge list",
            ));
        }
        let n = self.edges.len();
        let (Some(prev), Some(curr)) = (self.edges.back().copied(), self.edges.front().copied())
        else {
            return Err(super::malformed(
                "Remove command needs a non-empty edge list",
            ));
        };

        if prev.start == curr.end && n > 2 {
            let _ = self.edges.pop_front();
            let _ = self.edges.pop_back();
            let new_start = self
                .edges
                .front()
                .map(|edge| edge.start)
                .ok_or_else(|| super::malformed("Remove command needs a non-empty edge list"))?;
            let new_prev = self
                .edges
                .back_mut()
                .ok_or_else(|| super::malformed("Remove command needs a non-empty edge list"))?;
            new_prev.end = new_start;
            return Ok(());
        }

        let prev = self
            .edges
            .back_mut()
            .ok_or_else(|| super::malformed("Remove command needs a non-empty edge list"))?;
        prev.end = curr.end;
        let _ = self.edges.pop_front();
        Ok(())
    }

    fn process_command(
        &mut self,
        opcode: u8,
        reader: &mut CommandReader<'_>,
    ) -> Result<(), HpsError> {
        match opcode {
            0 => {
                let vertex = self.next_global_vertex()?;
                self.extend_current_edge(vertex)?;
                self.increase_edge_pointer(2)
            }
            1 => self.handle_previous(),
            2 => self.handle_next(),
            3 => self.increase_edge_pointer(1),
            4 => {
                let v0 = self.next_global_vertex()?;
                let v1 = self.next_global_vertex()?;
                let v2 = self.next_global_vertex()?;
                self.create_restart_face(v0, v1, v2)
            }
            5 => {
                let v0 = self.read_index_payload(reader)?;
                let v1 = self.read_index_payload(reader)?;
                let v2 = self.read_index_payload(reader)?;
                self.create_restart_face(v0, v1, v2)
            }
            6 => {
                let v0 = reader.read_u32()?;
                let v1 = reader.read_u32()?;
                let v2 = reader.read_u32()?;
                self.create_restart_face(v0, v1, v2)
            }
            7 => {
                let vertex = self.read_index_payload(reader)?;
                self.extend_current_edge(vertex)?;
                self.increase_edge_pointer(2)
            }
            8 => {
                let vertex = reader.read_u32()?;
                self.extend_current_edge(vertex)?;
                self.increase_edge_pointer(2)
            }
            9 => self.remove_current_edge(),
            10 => self.next_global_vertex().map(|_| ()),
            _ => Err(super::malformed("unknown face command opcode")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CommandReader, Edge, FaceDecoder, HpsError};

    struct VecReferenceDecoder {
        vertex_count: usize,
        index32: bool,
        global_vertex_ptr: u32,
        current_edge_idx: usize,
        edges: Vec<Edge>,
        faces: Vec<u32>,
    }

    impl VecReferenceDecoder {
        fn new(vertex_count: usize, index32: bool) -> Self {
            Self {
                vertex_count,
                index32,
                global_vertex_ptr: 0,
                current_edge_idx: 0,
                edges: Vec::new(),
                faces: Vec::new(),
            }
        }

        fn decode(&mut self, bytes: &[u8]) -> Result<Vec<u32>, HpsError> {
            let mut reader = CommandReader::new(bytes);
            while !reader.eof() {
                let command = reader.read_u8()?;
                if (command >> 4) != 0 {
                    return Err(malformed("face command high bits must be zero"));
                }
                self.process_command(command & 0x0f, &mut reader)?;
            }
            Ok(std::mem::take(&mut self.faces))
        }

        fn next_global_vertex(&mut self) -> Result<u32, HpsError> {
            let vertex = self.global_vertex_ptr;
            self.global_vertex_ptr = self
                .global_vertex_ptr
                .checked_add(1)
                .ok_or_else(|| malformed("face vertex pointer overflow"))?;
            self.validate(vertex)?;
            Ok(vertex)
        }

        fn validate(&self, vertex: u32) -> Result<(), HpsError> {
            let Ok(vertex) = usize::try_from(vertex) else {
                return Err(malformed("face vertex index is out of bounds"));
            };
            if vertex >= self.vertex_count {
                return Err(malformed("face vertex index is out of bounds"));
            }
            Ok(())
        }

        fn read_index_payload(&self, reader: &mut CommandReader<'_>) -> Result<u32, HpsError> {
            if self.index32 {
                reader.read_u32()
            } else {
                reader.read_u16()
            }
        }

        fn add_face(&mut self, a: u32, b: u32, c: u32) -> Result<(), HpsError> {
            self.validate(a)?;
            self.validate(b)?;
            self.validate(c)?;
            self.faces.extend_from_slice(&[a, b, c]);
            Ok(())
        }

        fn create_restart_face(&mut self, v0: u32, v1: u32, v2: u32) -> Result<(), HpsError> {
            self.add_face(v0, v1, v2)?;
            self.edges.clear();
            self.edges.push(Edge { start: v0, end: v1 });
            self.edges.push(Edge { start: v1, end: v2 });
            self.edges.push(Edge { start: v2, end: v0 });
            self.current_edge_idx = 0;
            Ok(())
        }

        fn extend_current_edge(&mut self, vertex: u32) -> Result<(), HpsError> {
            self.validate(vertex)?;
            if self.edges.is_empty() {
                return Err(malformed("face stream cannot extend an empty edge list"));
            }
            if self.current_edge_idx >= self.edges.len() {
                self.current_edge_idx = 0;
            }
            let current = self.edges[self.current_edge_idx];
            self.add_face(vertex, current.end, current.start)?;
            let _ = self.edges.splice(
                self.current_edge_idx..=self.current_edge_idx,
                [
                    Edge {
                        start: current.start,
                        end: vertex,
                    },
                    Edge {
                        start: vertex,
                        end: current.end,
                    },
                ],
            );
            Ok(())
        }

        fn increase_edge_pointer(&mut self, n: usize) -> Result<(), HpsError> {
            if self.edges.is_empty() {
                return Err(malformed("face stream cannot advance an empty edge list"));
            }
            self.current_edge_idx = (self.current_edge_idx + n) % self.edges.len();
            Ok(())
        }

        fn handle_previous(&mut self) -> Result<(), HpsError> {
            if self.edges.len() < 2 {
                return Err(malformed("Previous command needs at least two edges"));
            }
            let n = self.edges.len();
            let prev_idx = (self.current_edge_idx + n - 1) % n;
            let curr_idx = self.current_edge_idx;
            let prev = self.edges[prev_idx];
            let curr = self.edges[curr_idx];
            self.add_face(curr.start, prev.start, curr.end)?;

            let high = prev_idx.max(curr_idx);
            let low = prev_idx.min(curr_idx);
            self.edges.remove(high);
            self.edges.remove(low);
            self.edges.insert(
                low,
                Edge {
                    start: prev.start,
                    end: curr.end,
                },
            );
            self.current_edge_idx = (low + 1) % self.edges.len();
            Ok(())
        }

        fn handle_next(&mut self) -> Result<(), HpsError> {
            if self.edges.len() < 2 {
                return Err(malformed("Next command needs at least two edges"));
            }
            let curr_idx = self.current_edge_idx;
            let next_idx = (self.current_edge_idx + 1) % self.edges.len();
            let curr = self.edges[curr_idx];
            let next = self.edges[next_idx];
            self.add_face(curr.start, next.end, curr.end)?;

            let high = curr_idx.max(next_idx);
            let low = curr_idx.min(next_idx);
            self.edges.remove(high);
            self.edges.remove(low);
            self.edges.insert(
                low,
                Edge {
                    start: curr.start,
                    end: next.end,
                },
            );
            self.current_edge_idx = (low + 1) % self.edges.len();
            Ok(())
        }

        fn remove_current_edge(&mut self) -> Result<(), HpsError> {
            if self.edges.is_empty() {
                return Err(malformed("Remove command needs a non-empty edge list"));
            }
            let n = self.edges.len();
            let prev_idx = (self.current_edge_idx + n - 1) % n;
            let curr_idx = self.current_edge_idx;
            let prev = self.edges[prev_idx];
            let curr = self.edges[curr_idx];

            if prev.start == curr.end && n > 2 {
                let high = prev_idx.max(curr_idx);
                let low = prev_idx.min(curr_idx);
                self.edges.remove(high);
                self.edges.remove(low);
                if self.edges.is_empty() {
                    self.current_edge_idx = 0;
                } else {
                    let new_prev_idx = (low + self.edges.len() - 1) % self.edges.len();
                    let new_curr_idx = low % self.edges.len();
                    self.edges[new_prev_idx].end = self.edges[new_curr_idx].start;
                    self.current_edge_idx = new_curr_idx;
                }
                return Ok(());
            }

            self.edges[prev_idx].end = curr.end;
            self.edges.remove(curr_idx);
            self.current_edge_idx = if self.edges.is_empty() {
                0
            } else {
                curr_idx % self.edges.len()
            };
            Ok(())
        }

        fn process_command(
            &mut self,
            opcode: u8,
            reader: &mut CommandReader<'_>,
        ) -> Result<(), HpsError> {
            match opcode {
                0 => {
                    let vertex = self.next_global_vertex()?;
                    self.extend_current_edge(vertex)?;
                    self.increase_edge_pointer(2)
                }
                1 => self.handle_previous(),
                2 => self.handle_next(),
                3 => self.increase_edge_pointer(1),
                4 => {
                    let v0 = self.next_global_vertex()?;
                    let v1 = self.next_global_vertex()?;
                    let v2 = self.next_global_vertex()?;
                    self.create_restart_face(v0, v1, v2)
                }
                5 => {
                    let v0 = self.read_index_payload(reader)?;
                    let v1 = self.read_index_payload(reader)?;
                    let v2 = self.read_index_payload(reader)?;
                    self.create_restart_face(v0, v1, v2)
                }
                6 => {
                    let v0 = reader.read_u32()?;
                    let v1 = reader.read_u32()?;
                    let v2 = reader.read_u32()?;
                    self.create_restart_face(v0, v1, v2)
                }
                7 => {
                    let vertex = self.read_index_payload(reader)?;
                    self.extend_current_edge(vertex)?;
                    self.increase_edge_pointer(2)
                }
                8 => {
                    let vertex = reader.read_u32()?;
                    self.extend_current_edge(vertex)?;
                    self.increase_edge_pointer(2)
                }
                9 => self.remove_current_edge(),
                10 => self.next_global_vertex().map(|_| ()),
                _ => Err(malformed("unknown face command opcode")),
            }
        }
    }

    fn malformed(reason: impl Into<String>) -> HpsError {
        super::super::malformed(reason)
    }

    fn parse_with_vec_reference(
        bytes: &[u8],
        face_count: usize,
        vertex_count: usize,
    ) -> Result<Vec<u32>, HpsError> {
        let parse_mode = |index32| {
            let faces = VecReferenceDecoder::new(vertex_count, index32).decode(bytes)?;
            if faces.len() != face_count.saturating_mul(3) {
                return Err(malformed("face_count does not match decoded facets"));
            }
            Ok(faces)
        };
        parse_mode(false).or_else(|_| parse_mode(true))
    }

    fn compare_with_vec_reference(bytes: &[u8], face_count: usize, vertex_count: usize) {
        for index32 in [false, true] {
            let expected = VecReferenceDecoder::new(vertex_count, index32).decode(bytes);
            let actual = FaceDecoder::new(vertex_count, index32).decode(bytes);
            assert_eq!(
                actual, expected,
                "different decoding for index32={index32} and stream {bytes:?}"
            );
        }

        assert_eq!(
            super::parse(bytes, face_count, vertex_count),
            parse_with_vec_reference(bytes, face_count, vertex_count),
            "different parse/fallback result for stream {bytes:?}"
        );
    }

    #[test]
    fn deque_decoder_matches_vec_reference_for_short_facet_streams() {
        let streams: &[(&[u8], usize, usize)] = &[
            (&[], 0, 0),
            (&[4, 1, 2], 3, 32),
            (&[4, 9], 1, 32),
            (&[4, 0, 3, 0, 1, 2, 9, 4, 3], 6, 32),
            (&[6, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 3, 9], 1, 8),
            (&[5, 0, 0, 1, 0, 0, 0, 3, 9], 1, 8),
            (&[5, 0, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0], 1, 8),
            // Opcode 7 in 16-bit form, then a 32-bit form that must fall back.
            (&[4, 7, 3, 0], 2, 8),
            (&[4, 7, 3, 0, 0, 0], 2, 8),
            // Explicit 32-bit continuation and global-vertex skips.
            (&[4, 8, 3, 0, 0, 0], 2, 8),
            (&[10, 4, 10, 0], 2, 8),
            // Wrapped last/first Remove, followed by advance, extend, merge, remove.
            (
                &[6, 0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 9, 3, 0, 2, 9],
                3,
                8,
            ),
            (&[1], 0, 8),
            (&[2], 0, 8),
            (&[3], 0, 8),
            (&[9], 0, 8),
            (&[0], 0, 8),
            (&[0xf0], 0, 8),
            (&[11], 0, 8),
            (&[4], 0, 2),
            (&[6, 0, 0], 0, 8),
            (&[5, 0, 0], 0, 8),
        ];

        for &(bytes, face_count, vertex_count) in streams {
            compare_with_vec_reference(bytes, face_count, vertex_count);
        }
    }
}
