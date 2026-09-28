//! APU register logs: recording, exact replay, text round trip and typed errors.

use mos6502::apu::{Apu, LogError, LoggedWrite, RegisterLog};

fn lcg(seed: &mut u64) -> u64 {
    *seed = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    *seed >> 33
}

#[test]
fn random_sessions_replay_identically() {
    const REGS: [u16; 17] = [
        0, 1, 2, 3, 4, 5, 6, 7, 8, 0xA, 0xB, 0xC, 0xE, 0xF, 0x11, 0x15, 0x17,
    ];
    for s in 0..40u64 {
        let mut seed = s;
        let mut apu = Apu::new();
        let mut log = RegisterLog::default();
        let mut live = Vec::new();
        for _ in 0..60 {
            for _ in 0..lcg(&mut seed) % 4 {
                let reg = REGS[(lcg(&mut seed) % REGS.len() as u64) as usize];
                let mut value = lcg(&mut seed) as u8;
                if reg == 0x15 {
                    value &= 0x0F; // no DMC sample fetches: they are not in the log
                }
                log.write(&mut apu, reg, value);
            }
            for _ in 0..lcg(&mut seed) % 900 {
                apu.tick();
                live.push(apu.sample());
            }
        }
        let mut fresh = Apu::new();
        let replay = log.play(&mut fresh, live.len() as u64);
        assert_eq!(replay, live, "seed {s}");
        assert_eq!(fresh.cycles(), apu.cycles());
        assert_eq!(RegisterLog::parse(&log.to_text()).unwrap(), log);
    }
}

#[test]
fn play_resumes_mid_log() {
    let mut log = RegisterLog::default();
    let mut apu = Apu::new();
    log.write(&mut apu, 0x15, 0x01);
    log.write(&mut apu, 0x00, 0xBF);
    log.write(&mut apu, 0x02, 0x40);
    log.write(&mut apu, 0x03, 0x08);
    let whole = log.play(&mut Apu::new(), 2000);
    let mut b = Apu::new();
    let mut parts = log.play(&mut b, 700);
    parts.extend(log.play(&mut b, 1300));
    assert_eq!(parts, whole);
}

#[test]
fn parse_errors_and_comments() {
    let ok = RegisterLog::parse("# header\n0 15 0F\n\n10 00 bf  # pulse\n10 01 00\n").unwrap();
    assert_eq!(
        ok.writes[1],
        LoggedWrite {
            cycle: 10,
            reg: 0,
            value: 0xBF
        }
    );
    for (text, want) in [
        (
            "1 2",
            LogError::Syntax {
                line: 1,
                text: "1 2".into(),
            },
        ),
        (
            "x 00 00",
            LogError::Syntax {
                line: 1,
                text: "x 00 00".into(),
            },
        ),
        (
            "0 00 100",
            LogError::Syntax {
                line: 1,
                text: "0 00 100".into(),
            },
        ),
        (
            "0 00 00 00",
            LogError::Syntax {
                line: 1,
                text: "0 00 00 00".into(),
            },
        ),
        (
            "0 00 00\n1 18 00",
            LogError::BadRegister { line: 2, reg: 0x18 },
        ),
        (
            "5 00 00\n4 00 00",
            LogError::OutOfOrder {
                line: 2,
                cycle: 4,
                previous: 5,
            },
        ),
    ] {
        let e = RegisterLog::parse(text).unwrap_err();
        assert_eq!(e, want);
        assert!(!e.to_string().is_empty());
    }
}
