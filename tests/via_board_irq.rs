//! VIA board: timer 1 free-runs, every timeout interrupts, the handler toggles PA0.

use mos6502::systems::via_board::ViaComputer;

#[test]
fn pa0_toggles_once_per_timer_period_forever() {
    let mut pc = ViaComputer::new().unwrap();
    let mut edges = Vec::new();
    let mut last = pc.port_a() & 1;
    for cycle in 0..5000u32 {
        pc.run_cycles(1).unwrap();
        let now = pc.port_a() & 1;
        if now != last {
            edges.push(cycle);
            last = now;
        }
    }
    assert!(edges.len() >= 70, "{edges:?}");
    let gaps: Vec<u32> = edges.windows(2).map(|w| w[1] - w[0]).collect();
    // Latch $0040 free-runs with a period of N + 2 = 66 cycles, longer than the
    // handler, so every timeout is serviced and the gaps are exact.
    assert!(gaps.iter().all(|&g| g == 66), "{gaps:?}");
}
