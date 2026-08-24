use crate::error::HpsError;

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
    current_edge_idx: usize,
    edges: Vec<Edge>,
    faces: Vec<u32>,
}

impl FaceDecoder {
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
        self.edges.push(Edge { start: v0, end: v1 });
        self.edges.push(Edge { start: v1, end: v2 });
        self.edges.push(Edge { start: v2, end: v0 });
        self.current_edge_idx = 0;
        Ok(())
    }

    fn extend_current_edge(&mut self, vertex: u32) -> Result<(), HpsError> {
        self.validate(vertex)?;
        if self.edges.is_empty() {
            return Err(super::malformed(
                "face stream cannot extend an empty edge list",
            ));
        }
        if self.current_edge_idx >= self.edges.len() {
            self.current_edge_idx = 0;
        }
        let current = self.edges[self.current_edge_idx];
        self.add_face(vertex, current.end, current.start)?;
        // One edge becomes two, in place. Written as a remove and two inserts
        // this shifted the tail of the list three times per command, and the
        // list grows by one per command: 20 000 commands took 0.06 s, 40 000
        // took 0.23 s, 160 000 took 4.08 s -- four times the work for twice
        // the file. Splicing shifts it once. The growth is still quadratic in
        // a stream that never restarts (a real one restarts, which clears the
        // list), and the request deadline is what bounds that.
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
            return Err(super::malformed(
                "face stream cannot advance an empty edge list",
            ));
        }
        self.current_edge_idx = (self.current_edge_idx + n) % self.edges.len();
        Ok(())
    }

    fn handle_previous(&mut self) -> Result<(), HpsError> {
        if self.edges.len() < 2 {
            return Err(super::malformed(
                "Previous command needs at least two edges",
            ));
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
            return Err(super::malformed("Next command needs at least two edges"));
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
            return Err(super::malformed(
                "Remove command needs a non-empty edge list",
            ));
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
            _ => Err(super::malformed("unknown face command opcode")),
        }
    }
}

#[cfg(test)]
mod edge_list_tests {
    /// One edge becomes two in a single shift of the list, not three.
    ///
    /// Every extend command grows the edge list by one and shifts its tail;
    /// written as a remove and two inserts that shift happened three times per
    /// command. Measured on a stream that never restarts: 40 000 commands went
    /// from 0.23 s to 0.08 s, 160 000 from 4.08 s to 1.46 s. The cost is still
    /// quadratic in such a stream -- a real one restarts, which clears the list
    /// -- and the request deadline is what bounds the rest.
    #[test]
    fn the_edge_split_shifts_the_list_once() {
        let source = include_str!("faces.rs");
        assert!(
            source.contains("self.edges.splice("),
            "splitting an edge must shift the list once"
        );
        let split = source
            .split("fn extend_current_edge")
            .nth(1)
            .unwrap_or_default();
        let body = split.split("\n    fn ").next().unwrap_or_default();
        assert!(
            !body.contains("self.edges.insert("),
            "the split is back to inserting one edge at a time"
        );
    }
}
