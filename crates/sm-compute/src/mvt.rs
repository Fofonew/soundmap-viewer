//! Décodeur minimal de tuiles vectorielles Mapbox (MVT, protobuf), pur Rust,
//! limité à ce dont SoundMap a besoin : polygones et attributs texte.
//! Spécification : <https://github.com/mapbox/vector-tile-spec> (v2).

#[derive(Debug, Clone, PartialEq)]
pub struct MvtFeature {
    /// Anneaux en coordonnées de tuile (0..extent).
    pub rings: Vec<Vec<(f64, f64)>>,
    /// Attributs texte (clé, valeur).
    pub props: Vec<(String, String)>,
    pub geom_type: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MvtLayer {
    pub name: String,
    pub extent: u32,
    pub features: Vec<MvtFeature>,
}

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Reader<'a> {
    fn varint(&mut self) -> Option<u64> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let byte = *self.b.get(self.i)?;
            self.i += 1;
            v |= u64::from(byte & 0x7F) << shift;
            if byte & 0x80 == 0 {
                return Some(v);
            }
        }
        None
    }
    fn bytes(&mut self) -> Option<&'a [u8]> {
        let n = usize::try_from(self.varint()?).ok()?;
        let s = self.b.get(self.i..self.i + n)?;
        self.i += n;
        Some(s)
    }
    fn skip(&mut self, wire: u64) -> Option<()> {
        match wire {
            0 => {
                self.varint()?;
            }
            1 => self.i += 8,
            2 => {
                self.bytes()?;
            }
            5 => self.i += 4,
            _ => return None,
        }
        Some(())
    }
    fn done(&self) -> bool {
        self.i >= self.b.len()
    }
}

fn packed(b: &[u8]) -> Option<Vec<u32>> {
    let mut r = Reader { b, i: 0 };
    let mut out = Vec::new();
    while !r.done() {
        out.push(u32::try_from(r.varint()?).ok()?);
    }
    Some(out)
}

fn string_value(b: &[u8]) -> Option<String> {
    let mut r = Reader { b, i: 0 };
    while !r.done() {
        let key = r.varint()?;
        if key >> 3 == 1 && key & 7 == 2 {
            return Some(String::from_utf8_lossy(r.bytes()?).into_owned());
        }
        r.skip(key & 7)?;
    }
    None
}

fn zigzag(v: u32) -> i64 {
    i64::from(v >> 1) ^ -(i64::from(v & 1))
}

fn decode_geometry(cmds: &[u32]) -> Vec<Vec<(f64, f64)>> {
    let (mut x, mut y) = (0i64, 0i64);
    let mut rings = Vec::new();
    let mut cur: Vec<(f64, f64)> = Vec::new();
    let mut i = 0;
    while i < cmds.len() {
        let id = cmds[i] & 7;
        let count = (cmds[i] >> 3) as usize;
        i += 1;
        match id {
            1 | 2 => {
                for _ in 0..count {
                    let (Some(&dx), Some(&dy)) = (cmds.get(i), cmds.get(i + 1)) else {
                        break;
                    };
                    i += 2;
                    x += zigzag(dx);
                    y += zigzag(dy);
                    if id == 1 && !cur.is_empty() {
                        rings.push(std::mem::take(&mut cur));
                    }
                    cur.push((x as f64, y as f64));
                }
            }
            7 => {
                if let Some(&first) = cur.first() {
                    cur.push(first);
                }
                rings.push(std::mem::take(&mut cur));
            }
            _ => break,
        }
    }
    if !cur.is_empty() {
        rings.push(cur);
    }
    rings
}

/// Décode une tuile ; renvoie les couches.
pub fn decode(tile: &[u8]) -> Option<Vec<MvtLayer>> {
    let mut r = Reader { b: tile, i: 0 };
    let mut layers = Vec::new();
    while !r.done() {
        let key = r.varint()?;
        if key >> 3 == 3 && key & 7 == 2 {
            layers.push(decode_layer(r.bytes()?)?);
        } else {
            r.skip(key & 7)?;
        }
    }
    Some(layers)
}

fn decode_layer(b: &[u8]) -> Option<MvtLayer> {
    let mut r = Reader { b, i: 0 };
    let mut name = String::new();
    let mut extent = 4096;
    let mut raw_features: Vec<&[u8]> = Vec::new();
    let mut keys = Vec::new();
    let mut values = Vec::new();
    while !r.done() {
        let key = r.varint()?;
        match (key >> 3, key & 7) {
            (1, 2) => name = String::from_utf8_lossy(r.bytes()?).into_owned(),
            (2, 2) => raw_features.push(r.bytes()?),
            (3, 2) => keys.push(String::from_utf8_lossy(r.bytes()?).into_owned()),
            (4, 2) => values.push(string_value(r.bytes()?).unwrap_or_default()),
            (5, 0) => extent = u32::try_from(r.varint()?).ok()?,
            (_, w) => r.skip(w)?,
        }
    }
    let mut features = Vec::with_capacity(raw_features.len());
    for fb in raw_features {
        let mut fr = Reader { b: fb, i: 0 };
        let (mut tags, mut geom, mut gtype) = (Vec::new(), Vec::new(), 0);
        while !fr.done() {
            let key = fr.varint()?;
            match (key >> 3, key & 7) {
                (2, 2) => tags = packed(fr.bytes()?)?,
                (3, 0) => gtype = u32::try_from(fr.varint()?).ok()?,
                (4, 2) => geom = packed(fr.bytes()?)?,
                (_, w) => fr.skip(w)?,
            }
        }
        let props = tags
            .as_chunks::<2>()
            .0
            .iter()
            .filter_map(|kv| {
                Some((
                    keys.get(kv[0] as usize)?.clone(),
                    values.get(kv[1] as usize)?.clone(),
                ))
            })
            .collect();
        features.push(MvtFeature {
            rings: decode_geometry(&geom),
            props,
            geom_type: gtype,
        });
    }
    Some(MvtLayer {
        name,
        extent,
        features,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn varint(mut v: u64, out: &mut Vec<u8>) {
        loop {
            let b = (v & 0x7F) as u8;
            v >>= 7;
            if v == 0 {
                out.push(b);
                break;
            }
            out.push(b | 0x80);
        }
    }
    fn field_bytes(field: u64, data: &[u8], out: &mut Vec<u8>) {
        varint(field << 3 | 2, out);
        varint(data.len() as u64, out);
        out.extend_from_slice(data);
    }
    fn zz(v: i64) -> u64 {
        ((v << 1) ^ (v >> 63)) as u64
    }

    #[test]
    fn decodes_square_polygon_with_attribute() {
        // Carré (0,0)-(10,10), attribut code_cs = "CS2.2.1".
        let mut geom = Vec::new();
        for v in [
            1 << 3 | 1,
            zz(0),
            zz(0),
            3 << 3 | 2,
            zz(10),
            zz(0),
            zz(0),
            zz(10),
            zz(-10),
            zz(0),
            7 | 1 << 3,
        ] {
            varint(v, &mut geom);
        }
        let mut feat = Vec::new();
        field_bytes(2, &[0, 0], &mut feat); // tags packés : clé 0, valeur 0
        varint(3 << 3, &mut feat);
        varint(3, &mut feat); // type polygone
        field_bytes(4, &geom, &mut feat);
        let mut val = Vec::new();
        field_bytes(1, b"CS2.2.1", &mut val);
        let mut layer = Vec::new();
        field_bytes(1, b"occupation_sol", &mut layer);
        field_bytes(2, &feat, &mut layer);
        field_bytes(3, b"code_cs", &mut layer);
        field_bytes(4, &val, &mut layer);
        varint(5 << 3, &mut layer);
        varint(4096, &mut layer);
        let mut tile = Vec::new();
        field_bytes(3, &layer, &mut tile);

        let layers = decode(&tile).unwrap();
        assert_eq!(layers[0].name, "occupation_sol");
        let f = &layers[0].features[0];
        assert_eq!(f.props, vec![("code_cs".into(), "CS2.2.1".into())]);
        assert_eq!(f.rings[0].len(), 5);
        assert_eq!(f.rings[0][2], (10.0, 10.0));
    }
}
