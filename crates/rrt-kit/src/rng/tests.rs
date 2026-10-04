use super::*;

fn take<G: Generator>(g: &mut G, n: usize) -> Vec<u32> {
    (0..n).map(|_| g.next()).collect()
}

/// The C standard's example rand seeded with 1, as every compiler that
/// ships it prints.
#[test]
fn ansi_c_rand_matches_its_published_sequence() {
    assert_eq!(take(&mut Lcg::ANSI_C.seeded(1), 5), [16838, 5758, 10113, 17515, 31051]);
}

#[test]
fn msvc_rand_matches_its_published_sequence() {
    assert_eq!(take(&mut Lcg::MSVC.seeded(1), 5), [41, 18467, 6334, 26500, 19169]);
}

/// The reference pcg32-demo, `pcg32_srandom(42, 54)`.
#[test]
fn pcg32_matches_the_reference_demo() {
    let want = [0xa15c_02b7, 0x7b47_f409, 0xba1d_3330, 0x83d2_f293, 0xbfa4_784b, 0xcbed_606e];
    assert_eq!(take(&mut Pcg32::new(42, 54), 6), want);
}

#[test]
fn xorshift32_steps_as_marsaglia_wrote_it() {
    let mut x = Xorshift32::new(1);
    assert_eq!(x.next(), 270_369);
    assert_eq!(Xorshift32::new(0).state, 1, "0 would stay 0");
    let mut seen_zero = false;
    for _ in 0..100_000 {
        seen_zero |= x.next() == 0;
    }
    assert!(!seen_zero);
}

/// The textbook 16-bit Galois LFSR from 0xACE1 with taps 0xB400: first
/// 0xE270, and every non-zero state once in 65535 steps.
#[test]
fn a_maximal_16_bit_lfsr_has_period_65535() {
    let mut l = Lfsr::new(0xace1, 0xb400, 16);
    assert_eq!(l.next(), 0xe270);
    let mut steps = 1u32;
    while l.state != 0xace1 {
        l.next();
        steps += 1;
        assert!(steps <= 65535);
    }
    assert_eq!(steps, 65535);
    assert_eq!(l.bits(), 16);
}

#[test]
fn newlib_rand_is_its_64_bit_lcg_formula() {
    let mut g = Lcg64::NEWLIB.seeded(1);
    let state = 1u64.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    assert_eq!(g.next(), ((state >> 32) & 0x7fff_ffff) as u32);
    assert_eq!(g.bits(), 31);
}

#[test]
fn a_table_reads_in_turn_and_wraps() {
    let mut t = TableRng::new(&[10u8, 20, 30][..]);
    assert_eq!(take(&mut t, 4), [20, 30, 10, 20], "step first, as DOOM's P_Random");
    let mut t = TableRng { step_first: false, ..TableRng::new(vec![10u8, 20, 30]) };
    assert_eq!(take(&mut t, 4), [10, 20, 30, 10]);
}

/// `jump(n)` lands where `n` plain steps do, including a large `n` (the
/// stepping loop is side-effect free, so the optimizer may shorten it; it
/// still computes the same state).
#[test]
fn an_lcg_jumps_to_where_stepping_lands() {
    for n in [0u64, 1, 2, 3, 1000, 123_456_789] {
        let mut stepped = Lcg::ANSI_C.seeded(7);
        for _ in 0..n {
            stepped.state = stepped.state.wrapping_mul(stepped.mul).wrapping_add(stepped.add);
        }
        let mut jumped = Lcg::ANSI_C.seeded(7);
        jumped.jump(n);
        assert_eq!(jumped.state, stepped.state, "n = {n}");
    }
    let mut skipped = Lcg::MSVC;
    skipped.skip(500);
    let mut jumped = Lcg::MSVC;
    jumped.jump(500);
    assert_eq!(jumped, skipped, "jump agrees with skip, which calls next");
}

#[test]
fn saved_state_replays_the_same_sequence() {
    fn replay<G: Generator>(mut g: G) {
        g.skip(17);
        let saved = g.state();
        let first = take(&mut g, 20);
        g.set_state(saved);
        assert_eq!(take(&mut g, 20), first);
    }
    replay(Lcg::ANSI_C);
    replay(Lcg64::NEWLIB);
    replay(Xorshift32::new(99));
    replay(Lfsr::new(0xace1, 0xb400, 16));
    replay(TableRng::new(vec![1u8, 2, 3, 4, 5]));
    replay(Pcg32::new(1, 2));
    replay(Counted::new(Lcg::MSVC));
}

/// `modulo` and `scaled` are the expressions games wrote, value for value.
#[test]
fn the_era_reductions_are_the_expressions_they_name() {
    let (mut a, mut b) = (Lcg::ANSI_C, Lcg::ANSI_C);
    for _ in 0..1000 {
        assert_eq!(a.modulo(6), b.next() % 6);
        let v = b.next();
        assert_eq!(a.scaled(6), (v * 6) >> 15);
        let v = b.next();
        assert_eq!(a.top_bits(4), v >> 11);
    }
    let mut g = Lcg::ANSI_C;
    assert_eq!(g.top_bits(0), 0);
}

#[test]
fn below_and_range_cover_their_span_and_stay_inside() {
    let mut g = Pcg32::new(5, 5);
    let mut hits = [0u32; 7];
    for _ in 0..7000 {
        hits[g.below(7) as usize] += 1;
        let r = g.range(-3, 3);
        assert!((-3..=3).contains(&r));
    }
    assert!(hits.iter().all(|h| *h > 800), "{hits:?}");
    let mut tiny = Lfsr::new(1, 0b110, 3);
    for _ in 0..100 {
        assert!(tiny.below(5) < 5, "rejection keeps a 3-bit generator in range");
    }
    assert_eq!(g.range(4, 4), 4);
}

#[test]
fn chance_pick_shuffle_and_unit() {
    let mut g = Pcg32::new(9, 1);
    assert!(!(0..100).any(|_| g.chance(0, 4)));
    assert!((0..100).all(|_| g.chance(4, 4)));
    let heads = (0..10_000).filter(|_| g.chance(1, 2)).count();
    assert!((4500..5500).contains(&heads), "{heads}");
    assert_eq!(g.pick::<u8>(&[]), None);
    assert!([1, 2, 3].contains(g.pick(&[1, 2, 3]).unwrap()));
    let mut deck: Vec<u32> = (0..52).collect();
    g.shuffle(&mut deck);
    let mut sorted = deck.clone();
    sorted.sort();
    assert_eq!(sorted, (0..52).collect::<Vec<_>>());
    assert_ne!(deck, sorted, "a 52-card shuffle left in order is a 1 in 52! event");
    for _ in 0..1000 {
        let u = g.unit_f32();
        assert!((0.0..1.0).contains(&u));
    }
}

#[test]
fn counted_counts_and_recorded_remembers() {
    let mut c = Counted::new(Lcg::ANSI_C);
    c.skip(3);
    c.modulo(10);
    assert_eq!(c.calls, 4);
    let mut r = Recorded::new(Lcg::ANSI_C.seeded(1), 3);
    take(&mut r, 5);
    assert_eq!(r.history.iter().copied().collect::<Vec<_>>(), [10113, 17515, 31051]);
}

/// Forced values come out first and do not step the inner generator.
#[test]
fn forced_values_come_first_and_leave_the_sequence_alone() {
    let mut f = Forced::new(Lcg::ANSI_C.seeded(1));
    f.force([0, 32767]);
    assert_eq!(f.queued(), 2);
    assert_eq!(take(&mut f, 4), [0, 32767, 16838, 5758]);
    assert_eq!(f.bits(), 15);
}

/// A game's own scheme: implement `Generator`, and the reductions come
/// with it.
#[test]
fn a_custom_generator_gets_every_helper() {
    /// A made-up 8-bit scheme: add 37, rotate left 3.
    struct Mine(u8);
    impl Generator for Mine {
        type State = u8;
        fn next(&mut self) -> u32 {
            self.0 = self.0.wrapping_add(37).rotate_left(3);
            u32::from(self.0)
        }
        fn bits(&self) -> u32 {
            8
        }
        fn state(&self) -> u8 {
            self.0
        }
        fn set_state(&mut self, s: u8) {
            self.0 = s;
        }
    }
    let mut m = Mine(0);
    assert_eq!(m.next(), 37u8.rotate_left(3) as u32);
    assert!(m.below(10) < 10);
    assert!(m.scaled(4) < 4);
    let mut dynamic: Box<dyn Generator<State = u8>> = Box::new(Mine(1));
    assert!(dynamic.modulo(3) < 3, "works through a trait object too");
}
