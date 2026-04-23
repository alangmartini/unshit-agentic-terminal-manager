use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const TRANSPARENT: Color = Color {
        r: 0,
        g: 0,
        b: 0,
        a: 0,
    };
    pub const BLACK: Color = Color {
        r: 0,
        g: 0,
        b: 0,
        a: 255,
    };
    pub const WHITE: Color = Color {
        r: 255,
        g: 255,
        b: 255,
        a: 255,
    };

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }
}

pub const ANSI_16: [Color; 16] = [
    Color {
        r: 0,
        g: 0,
        b: 0,
        a: 255,
    },
    Color {
        r: 170,
        g: 0,
        b: 0,
        a: 255,
    },
    Color {
        r: 0,
        g: 170,
        b: 0,
        a: 255,
    },
    Color {
        r: 170,
        g: 85,
        b: 0,
        a: 255,
    },
    Color {
        r: 0,
        g: 0,
        b: 170,
        a: 255,
    },
    Color {
        r: 170,
        g: 0,
        b: 170,
        a: 255,
    },
    Color {
        r: 0,
        g: 170,
        b: 170,
        a: 255,
    },
    Color {
        r: 170,
        g: 170,
        b: 170,
        a: 255,
    },
    Color {
        r: 85,
        g: 85,
        b: 85,
        a: 255,
    },
    Color {
        r: 255,
        g: 85,
        b: 85,
        a: 255,
    },
    Color {
        r: 85,
        g: 255,
        b: 85,
        a: 255,
    },
    Color {
        r: 255,
        g: 255,
        b: 85,
        a: 255,
    },
    Color {
        r: 85,
        g: 85,
        b: 255,
        a: 255,
    },
    Color {
        r: 255,
        g: 85,
        b: 255,
        a: 255,
    },
    Color {
        r: 85,
        g: 255,
        b: 255,
        a: 255,
    },
    Color {
        r: 255,
        g: 255,
        b: 255,
        a: 255,
    },
];

pub fn color_256(index: u8) -> Color {
    if index < 16 {
        return ANSI_16[index as usize];
    }
    if index < 232 {
        let idx = (index - 16) as u16;
        let b_idx = idx % 6;
        let g_idx = (idx / 6) % 6;
        let r_idx = idx / 36;
        let to_val = |i: u16| -> u8 {
            if i == 0 {
                0
            } else {
                (55 + 40 * i) as u8
            }
        };
        Color::rgb(to_val(r_idx), to_val(g_idx), to_val(b_idx))
    } else {
        let v = 8 + 10 * (index - 232) as u16;
        Color::rgb(v as u8, v as u8, v as u8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_match_spec() {
        assert_eq!(
            Color::TRANSPARENT,
            Color {
                r: 0,
                g: 0,
                b: 0,
                a: 0
            }
        );
        assert_eq!(
            Color::BLACK,
            Color {
                r: 0,
                g: 0,
                b: 0,
                a: 255
            }
        );
        assert_eq!(
            Color::WHITE,
            Color {
                r: 255,
                g: 255,
                b: 255,
                a: 255
            }
        );
    }

    #[test]
    fn ansi16_red_and_bright_red() {
        assert_eq!(
            ANSI_16[1],
            Color {
                r: 170,
                g: 0,
                b: 0,
                a: 255
            }
        );
        assert_eq!(
            ANSI_16[9],
            Color {
                r: 255,
                g: 85,
                b: 85,
                a: 255
            }
        );
    }

    #[test]
    fn color_256_first_cube_entry_is_black() {
        assert_eq!(color_256(16), Color::rgb(0, 0, 0));
    }

    #[test]
    fn color_256_delegates_to_ansi16_for_low_indices() {
        for i in 0..16u8 {
            assert_eq!(color_256(i), ANSI_16[i as usize]);
        }
    }

    #[test]
    fn color_256_cube_sample_196_is_red() {
        // 196 = 16 + 5*36, which is the top-right red corner of the cube.
        assert_eq!(color_256(196), Color::rgb(255, 0, 0));
    }

    #[test]
    fn color_256_grayscale_ramp_endpoints() {
        assert_eq!(color_256(232), Color::rgb(8, 8, 8));
        assert_eq!(color_256(255), Color::rgb(238, 238, 238));
    }

    #[test]
    fn color_round_trips_through_serde_json() {
        let c = Color {
            r: 10,
            g: 20,
            b: 30,
            a: 40,
        };
        let j = serde_json::to_string(&c).unwrap();
        let back: Color = serde_json::from_str(&j).unwrap();
        assert_eq!(back, c);
    }
}
