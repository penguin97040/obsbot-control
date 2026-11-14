//! Platform-independent encoding and decoding of the OBSBOT UVC extension-unit (XU)
//! messages. See docs/PROTOCOL.md for the source of every constant.

pub const VENDOR_ID: u16 = 0x3564;
pub const PID_TINY2: u16 = 0xFEF8;
pub const PID_TINY2_LITE: u16 = 0xFEF9;

/// Every XU control on the vendor unit transfers exactly 60 bytes.
pub const XU_LENGTH: usize = 60;
pub type XuBuffer = [u8; XU_LENGTH];
/// The largest payload a framed command can carry.
pub const MAX_PAYLOAD: usize = XU_LENGTH - 16;

/// CRC-framed "V3" commands (SET) and their reply mailbox (GET).
pub const SELECTOR_FRAMED: u8 = 2;
/// Simple tag commands (SET) and the 60-byte status block (GET).
pub const SELECTOR_SIMPLE: u8 = 6;
pub const SELECTOR_PRODUCT_NAME: u8 = 8;

/// The OBSBOT extension unit's GUID, {9A1E7291-6843-4683-6D92-39BC7906EE49}, as the
/// 16 bytes that appear in a USB descriptor (first three fields little-endian).
pub const XU_GUID_BYTES: [u8; 16] = [
    0x91, 0x72, 0x1E, 0x9A, 0x43, 0x68, 0x83, 0x46, 0x6D, 0x92, 0x39, 0xBC, 0x79, 0x06, 0xEE, 0x49,
];
/// The unit ID the published sources report for the extension unit.
pub const XU_DEFAULT_UNIT: u8 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Model {
    #[default]
    Unknown,
    Tiny2,
    Tiny2Lite,
}

impl Model {
    pub fn from_pid(pid: u16) -> Model {
        match pid {
            PID_TINY2 => Model::Tiny2,
            PID_TINY2_LITE => Model::Tiny2Lite,
            _ => Model::Unknown,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Model::Tiny2 => "OBSBOT Tiny 2",
            Model::Tiny2Lite => "OBSBOT Tiny 2 Lite",
            Model::Unknown => "Unknown OBSBOT camera",
        }
    }
}

/// Which byte codes to use where the published sources disagree.
/// Tiny2 = lxman/obsbot-mcp and cgevans/tiny2 (tested on a Tiny 2).
/// Lite = me-tony/nod (tested on a Tiny 2 Lite).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Variant {
    #[default]
    Tiny2,
    Lite,
}

impl Variant {
    pub fn label(self) -> &'static str {
        match self {
            Variant::Tiny2 => "Tiny 2",
            Variant::Lite => "Tiny 2 Lite",
        }
    }
}

/// CRC-16/USB: polynomial 0x8005 reflected (0xA001), init 0xFFFF, xorout 0xFFFF.
pub fn crc16_usb(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &byte in data {
        crc ^= byte as u16;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xA001 } else { crc >> 1 };
        }
    }
    crc ^ 0xFFFF
}

fn put_u16(buf: &mut [u8], value: u16) {
    buf[..2].copy_from_slice(&value.to_le_bytes());
}

fn get_u16(buf: &[u8]) -> u16 {
    u16::from_le_bytes([buf[0], buf[1]])
}

// ---- Selector 6: simple tag commands, zero padded, no CRC ------------------------

/// Copies up to 60 bytes into a zero-padded buffer.
pub fn build_simple(bytes: &[u8]) -> XuBuffer {
    let mut buf = [0u8; XU_LENGTH];
    let n = bytes.len().min(XU_LENGTH);
    buf[..n].copy_from_slice(&bytes[..n]);
    buf
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AiMode {
    Off,
    Normal,
    UpperBody,
    CloseUp,
    Headless,
    LowerBody,
    Group,
    Hand,
    Whiteboard,
    Desk,
}

impl AiMode {
    pub const ALL: [AiMode; 10] = [
        AiMode::Off,
        AiMode::Normal,
        AiMode::UpperBody,
        AiMode::CloseUp,
        AiMode::Headless,
        AiMode::LowerBody,
        AiMode::Group,
        AiMode::Hand,
        AiMode::Whiteboard,
        AiMode::Desk,
    ];

    /// The two bytes (m, n) that select this mode.
    pub fn bytes(self) -> (u8, u8) {
        match self {
            AiMode::Off => (0x00, 0x00),
            AiMode::Normal => (0x02, 0x00),
            AiMode::UpperBody => (0x02, 0x01),
            AiMode::CloseUp => (0x02, 0x02),
            AiMode::Headless => (0x02, 0x03),
            AiMode::LowerBody => (0x02, 0x04),
            AiMode::Group => (0x01, 0x00),
            AiMode::Hand => (0x03, 0x00),
            AiMode::Whiteboard => (0x04, 0x00),
            AiMode::Desk => (0x05, 0x00),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            AiMode::Off => "Off",
            AiMode::Normal => "Normal (whole body)",
            AiMode::UpperBody => "Upper body",
            AiMode::CloseUp => "Close-up",
            AiMode::Headless => "Headless",
            AiMode::LowerBody => "Lower body",
            AiMode::Group => "Group",
            AiMode::Hand => "Hand",
            AiMode::Whiteboard => "Whiteboard",
            AiMode::Desk => "Desk",
        }
    }

    /// The mode these status bytes describe, or None when they are not a known mode.
    pub fn from_bytes(m: u8, n: u8) -> Option<AiMode> {
        AiMode::ALL.into_iter().find(|mode| mode.bytes() == (m, n))
    }

    pub fn command(self) -> XuBuffer {
        let (m, n) = self.bytes();
        build_simple(&[0x16, 0x02, m, n])
    }
}

pub fn hdr_command(on: bool) -> XuBuffer {
    build_simple(&[0x01, 0x01, on as u8])
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fov {
    Wide,
    Medium,
    Narrow,
}

impl Fov {
    pub const ALL: [Fov; 3] = [Fov::Wide, Fov::Medium, Fov::Narrow];

    pub fn name(self) -> &'static str {
        match self {
            Fov::Wide => "Wide",
            Fov::Medium => "Medium",
            Fov::Narrow => "Narrow",
        }
    }

    fn code(self, variant: Variant) -> u8 {
        let base = if variant == Variant::Lite { 1 } else { 0 };
        base + self as u8
    }

    pub fn command(self, variant: Variant) -> XuBuffer {
        build_simple(&[0x04, 0x01, self.code(variant)])
    }

    /// Maps the raw status byte back to a field of view (None for "custom").
    pub fn from_status(raw: u8, variant: Variant) -> Option<Fov> {
        Fov::ALL.into_iter().find(|f| f.code(variant) == raw)
    }
}

/// Sleep or wake with the simple command (the Lite variant).
pub fn simple_sleep_command(sleep: bool) -> XuBuffer {
    build_simple(&[0x02, 0x01, sleep as u8])
}

/// Recentre with the simple command (the Lite variant).
pub fn simple_recentre_command() -> XuBuffer {
    build_simple(&[0x16, 0x01, 0x00, 0x00])
}

// ---- Selector 2: CRC-framed "V3" commands ----------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Receiver {
    Camera = 0x02,
    Gimbal = 0x03,
    Ai = 0x04,
}

pub mod cmd {
    /// Camera, u32: 0 wake, 1 sleep.
    pub const SLEEP_WAKE: u16 = 0xA0C2;
    /// Gimbal, 6 zero bytes.
    pub const RECENTRE: u16 = 0x00C3;
    /// AI, 3 × f32 [roll, pitch, yaw] in degrees per second.
    pub const GIMBAL_SPEED: u16 = 0x6484;
    /// AI, u8: 0 standard, 2 sport.
    pub const TRACKING_SPEED: u16 = 0x0CC4;
}

pub const FLAG_SET: u8 = 0x25;
pub const FLAG_GET: u8 = 0x01;
pub const HOST_ID: u8 = 0x0A;

/// Builds a framed command. An empty payload makes a header-only GET frame. An
/// oversized payload returns an all-zero buffer.
pub fn build_frame(seq: u16, receiver: Receiver, command: u16, payload: &[u8]) -> XuBuffer {
    let mut f = [0u8; XU_LENGTH];
    if payload.len() > MAX_PAYLOAD {
        return f;
    }
    f[0] = 0xAA;
    f[1] = if payload.is_empty() { FLAG_GET } else { FLAG_SET };
    put_u16(&mut f[2..], seq);
    put_u16(&mut f[4..], 0x000C);
    f[8] = HOST_ID;
    f[9] = receiver as u8;
    put_u16(&mut f[10..], command);
    if !payload.is_empty() {
        put_u16(&mut f[12..], payload.len() as u16);
        f[16..16 + payload.len()].copy_from_slice(payload);
        // The payload CRC covers [12, 16 + len) with its own slot (14-15) zeroed.
        let crc = crc16_usb(&f[12..16 + payload.len()]);
        put_u16(&mut f[14..], crc);
    }
    // The header CRC covers [0, 12) with its own slot (6-7) zeroed.
    let crc = crc16_usb(&f[0..12]);
    put_u16(&mut f[6..], crc);
    f
}

pub fn sleep_wake_frame(seq: u16, sleep: bool) -> XuBuffer {
    build_frame(seq, Receiver::Camera, cmd::SLEEP_WAKE, &[sleep as u8, 0, 0, 0])
}

pub fn recentre_frame(seq: u16) -> XuBuffer {
    build_frame(seq, Receiver::Gimbal, cmd::RECENTRE, &[0; 6])
}

pub fn gimbal_speed_payload(pitch_deg_per_sec: f32, yaw_deg_per_sec: f32) -> [u8; 12] {
    let mut payload = [0u8; 12];
    payload[0..4].copy_from_slice(&0.0f32.to_le_bytes()); // roll
    payload[4..8].copy_from_slice(&pitch_deg_per_sec.to_le_bytes());
    payload[8..12].copy_from_slice(&yaw_deg_per_sec.to_le_bytes());
    payload
}

pub fn gimbal_speed_frame(seq: u16, pitch_deg_per_sec: f32, yaw_deg_per_sec: f32) -> XuBuffer {
    build_frame(
        seq,
        Receiver::Ai,
        cmd::GIMBAL_SPEED,
        &gimbal_speed_payload(pitch_deg_per_sec, yaw_deg_per_sec),
    )
}

pub fn tracking_speed_frame(seq: u16, sport: bool) -> XuBuffer {
    build_frame(seq, Receiver::Ai, cmd::TRACKING_SPEED, &[if sport { 2 } else { 0 }])
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FrameReply {
    pub flags: u8,
    pub seq: u16,
    pub command: u16,
    pub payload: Vec<u8>,
}

/// Checks the magic byte, header CRC, payload length and payload CRC of a reply.
pub fn parse_frame(buf: &XuBuffer) -> Option<FrameReply> {
    if buf[0] != 0xAA {
        return None;
    }
    let mut copy = *buf;
    copy[6] = 0;
    copy[7] = 0;
    if crc16_usb(&copy[0..12]) != get_u16(&buf[6..]) {
        return None;
    }
    let len = get_u16(&buf[12..]) as usize;
    if len > MAX_PAYLOAD {
        return None;
    }
    if len > 0 {
        copy[14] = 0;
        copy[15] = 0;
        if crc16_usb(&copy[12..16 + len]) != get_u16(&buf[14..]) {
            return None;
        }
    }
    Some(FrameReply {
        flags: buf[1],
        seq: get_u16(&buf[2..]),
        command: get_u16(&buf[10..]),
        payload: buf[16..16 + len].to_vec(),
    })
}

// ---- Status block (GET selector 6) ------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Status {
    /// False when the block was all zeros.
    pub valid: bool,
    pub asleep: bool,
    /// 1 run, 3 sleep, 4 privacy (cgevans).
    pub device_status: u8,
    pub hdr: bool,
    pub fov_raw: u8,
    pub zoom_percent: u16,
    pub ai_m: u8,
    pub ai_n: u8,
    /// None when the AI bytes are not a known mode.
    pub ai_mode: Option<AiMode>,
    pub sport_tracking: bool,
}

pub fn parse_status(buf: &XuBuffer) -> Status {
    if buf.iter().all(|&b| b == 0) {
        return Status::default();
    }
    let device_status = buf[0x09];
    Status {
        valid: true,
        device_status,
        asleep: buf[0x02] == 1 || device_status == 3,
        zoom_percent: get_u16(&buf[0x04..]),
        hdr: buf[0x06] != 0,
        fov_raw: buf[0x11],
        ai_m: buf[0x18],
        ai_n: buf[0x1C],
        ai_mode: AiMode::from_bytes(buf[0x18], buf[0x1C]),
        sport_tracking: buf[0x24] == 2,
    }
}

// ---- Hex helpers ------------------------------------------------------------------

/// "aa 25 01 ..." in lower case.
pub fn to_hex(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len() * 3);
    for (i, b) in data.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Parses "aa 25 0x01,ff" style input. Returns None on characters that are not hex
/// digits or separators (space, comma, colon, hyphen).
pub fn parse_hex(text: &str) -> Option<Vec<u8>> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut nibble: Option<u8> = None;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '0' && i + 1 < chars.len() && (chars[i + 1] == 'x' || chars[i + 1] == 'X') {
            if nibble.is_some() {
                return None;
            }
            i += 2;
            continue;
        }
        if c.is_whitespace() || c == ',' || c == ':' || c == '-' {
            if let Some(n) = nibble.take() {
                out.push(n);
            }
            i += 1;
            continue;
        }
        let v = c.to_digit(16)? as u8;
        match nibble.take() {
            None => nibble = Some(v),
            Some(high) => out.push((high << 4) | v),
        }
        i += 1;
    }
    if let Some(n) = nibble {
        out.push(n);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Expected frames are real USB captures published by cgevans/tiny2 and
    // mitchelloharawild/obsbot-tiny-2-control (see docs/PROTOCOL.md).

    /// The buffer must start with `expected_hex` and be all zeros after it.
    fn check_prefix(actual: &XuBuffer, expected_hex: &str, name: &str) {
        let expected = parse_hex(expected_hex).unwrap();
        assert_eq!(&actual[..expected.len()], &expected[..], "{name}: expected {expected_hex}, actual {}", to_hex(&actual[..expected.len()]));
        assert!(actual[expected.len()..].iter().all(|&b| b == 0), "{name}: trailing bytes not zero");
    }

    #[test]
    fn crc16_usb_check_value() {
        assert_eq!(crc16_usb(b"123456789"), 0xB4C8);
    }

    #[test]
    fn captured_framed_commands() {
        check_prefix(&sleep_wake_frame(0xA5, false), "aa 25 a5 00 0c 00 5f ef 0a 02 c2 a0 04 00 be 07 00 00 00 00", "wake seq a5");
        check_prefix(&sleep_wake_frame(0xAC, false), "aa 25 ac 00 0c 00 8f c0 0a 02 c2 a0 04 00 be 07 00 00 00 00", "wake seq ac");
        check_prefix(&sleep_wake_frame(0xAB, true), "aa 25 ab 00 0c 00 3e 1a 0a 02 c2 a0 04 00 bf fb 01 00 00 00", "sleep seq ab");
        check_prefix(
            &build_frame(0x16, Receiver::Camera, 0x2982, &[0x02, 0x04, 0x00, 0x00, 0x00]),
            "aa 25 16 00 0c 00 58 91 0a 02 82 29 05 00 b2 af 02 04",
            "auto exposure",
        );
        check_prefix(
            &build_frame(0x15, Receiver::Camera, 0x2982, &[0x01, 0x32, 0x00, 0x00, 0x00]),
            "aa 25 15 00 0c 00 a8 9e 0a 02 82 29 05 00 f9 27 01 32",
            "manual exposure",
        );
    }

    #[test]
    fn header_only_get_frame() {
        let f = build_frame(7, Receiver::Camera, 0x2942, &[]);
        assert_eq!(f[1], FLAG_GET);
        assert!(f[12] == 0 && f[13] == 0);
        let r = parse_frame(&f).unwrap();
        assert!(r.seq == 7 && r.command == 0x2942 && r.payload.is_empty());
    }

    #[test]
    fn round_trip_through_the_parser() {
        let r = parse_frame(&gimbal_speed_frame(0x1234, 10.0, -20.0)).unwrap();
        assert_eq!(r.seq, 0x1234);
        assert_eq!(r.command, cmd::GIMBAL_SPEED);
        assert_eq!(r.payload.len(), 12);
        let mut corrupt = gimbal_speed_frame(1, 0.0, 0.0);
        corrupt[10] ^= 0xFF;
        assert!(parse_frame(&corrupt).is_none());
    }

    #[test]
    fn payload_integrity_and_length() {
        let mut frame = sleep_wake_frame(0xA5, false);
        assert!(parse_frame(&frame).is_some());
        frame[16] ^= 1;
        assert!(parse_frame(&frame).is_none());
        frame = sleep_wake_frame(0xA5, false);
        frame[14] ^= 1;
        assert!(parse_frame(&frame).is_none());
        frame = sleep_wake_frame(0xA5, false);
        frame[12] = 45;
        assert!(parse_frame(&frame).is_none());
        frame[12] = 0xFF;
        frame[13] = 0xFF;
        assert!(parse_frame(&frame).is_none());

        let mut payload = vec![0xA5u8; 44];
        assert!(parse_frame(&build_frame(1, Receiver::Camera, 1, &payload)).is_some());
        payload.push(0);
        assert!(parse_frame(&build_frame(1, Receiver::Camera, 1, &payload)).is_none());

        // All captured command bytes continue to pass the stricter parser.
        for f in [sleep_wake_frame(0xAB, true), recentre_frame(1), tracking_speed_frame(2, true)] {
            assert!(parse_frame(&f).is_some());
        }
    }

    #[test]
    fn simple_tag_commands() {
        check_prefix(&AiMode::Off.command(), "16 02 00 00", "AI off");
        check_prefix(&AiMode::UpperBody.command(), "16 02 02 01", "AI upper body");
        check_prefix(&AiMode::Group.command(), "16 02 01 00", "AI group");
        check_prefix(&AiMode::Desk.command(), "16 02 05 00", "AI desk");
        check_prefix(&hdr_command(true), "01 01 01", "HDR on");
        check_prefix(&Fov::Wide.command(Variant::Tiny2), "04 01 00", "FOV wide tiny2");
        check_prefix(&Fov::Narrow.command(Variant::Lite), "04 01 03", "FOV narrow lite");
    }

    #[test]
    fn ai_mode_bytes_round_trip() {
        for mode in AiMode::ALL {
            let b = mode.command();
            assert_eq!(AiMode::from_bytes(b[2], b[3]), Some(mode));
        }
        assert_eq!(AiMode::from_bytes(0x09, 0x09), None);
    }

    #[test]
    fn status_parsing() {
        let mut s: XuBuffer = [0; XU_LENGTH];
        assert!(!parse_status(&s).valid);
        s[0x02] = 1;
        s[0x06] = 1;
        s[0x04] = 50;
        s[0x11] = 2;
        s[0x18] = 2;
        s[0x1C] = 1;
        s[0x24] = 2;
        let st = parse_status(&s);
        assert!(st.valid && st.asleep && st.hdr && st.zoom_percent == 50);
        assert!(st.ai_mode == Some(AiMode::UpperBody) && st.sport_tracking);
        assert_eq!(Fov::from_status(st.fov_raw, Variant::Tiny2), Some(Fov::Narrow));
        assert_eq!(Fov::from_status(st.fov_raw, Variant::Lite), Some(Fov::Medium));
    }

    #[test]
    fn hex_parsing() {
        let v = parse_hex("0xAA, 25:0c-1 ff").unwrap();
        assert!(v.len() == 5 && v[0] == 0xAA && v[3] == 0x01 && v[4] == 0xFF);
        assert!(parse_hex("zz").is_none());
    }
}
