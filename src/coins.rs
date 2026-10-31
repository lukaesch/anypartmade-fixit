//! Reference coins. The coin in the photo sets the scale: its known diameter in mm
//! divided by its diameter in pixels gives mm per pixel.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coin {
    pub id: &'static str,
    pub label: &'static str,
    pub diameter_mm: f64,
}

pub const COINS: [Coin; 4] = [
    Coin {
        id: "eur_1",
        label: "1 euro",
        diameter_mm: 23.25,
    },
    Coin {
        id: "eur_2",
        label: "2 euro",
        diameter_mm: 25.75,
    },
    Coin {
        id: "eur_050",
        label: "50 cent",
        diameter_mm: 24.25,
    },
    Coin {
        id: "usd_025",
        label: "US quarter",
        diameter_mm: 24.26,
    },
];

pub fn find(id: &str) -> Option<Coin> {
    COINS.iter().copied().find(|c| c.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coins_are_found_by_id() {
        assert_eq!(find("eur_2").map(|c| c.diameter_mm), Some(25.75));
        assert_eq!(find("usd_025").map(|c| c.label), Some("US quarter"));
        assert!(find("eur_5").is_none());
    }
}
