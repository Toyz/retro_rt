//! Rumble probe: drives the pad's motors in labelled phases, first through
//! gilrs directly, then through `Input::rumble`, printing what each step
//! did. Feel which phases buzz:
//!
//! ```text
//! cargo run -p rrt-input --example rumble
//! ```

use std::time::{Duration, Instant};

use gilrs::ff::{BaseEffect, BaseEffectType, EffectBuilder, Replay, Ticks};
use rrt_input::{Input, Motors};

fn wait(input: &mut Input, secs: f32) {
    let end = Instant::now() + Duration::from_secs_f32(secs);
    while Instant::now() < end {
        input.read();
        std::thread::sleep(Duration::from_millis(16));
    }
}

fn main() {
    tracing_subscriber::fmt().with_env_filter("debug").init();

    println!("== gamepads gilrs sees");
    let mut g = gilrs::GilrsBuilder::new().with_default_filters(false).build().expect("gilrs");
    while g.next_event().is_some() {}
    let mut ff_pad = None;
    for (id, p) in g.gamepads() {
        println!(
            "  {id}: {:?} connected {} ff {} power {:?}",
            p.name(),
            p.is_connected(),
            p.is_ff_supported(),
            p.power_info()
        );
        if p.is_connected() && p.is_ff_supported() {
            ff_pad.get_or_insert(id);
        }
    }

    match ff_pad {
        None => println!("== no connected pad reports force feedback: gilrs cannot rumble anything"),
        Some(id) => {
            for (label, kind) in [
                ("strong (large) motor, full", BaseEffectType::Strong { magnitude: u16::MAX }),
                ("weak (small) motor, full", BaseEffectType::Weak { magnitude: u16::MAX }),
            ] {
                println!("== phase A: gilrs directly, {label}, 1.5 s");
                let effect = EffectBuilder::new()
                    .add_effect(BaseEffect {
                        kind,
                        scheduling: Replay { play_for: Ticks::from_ms(1500), ..Default::default() },
                        ..Default::default()
                    })
                    .gamepads(&[id])
                    .finish(&mut g);
                match effect {
                    Ok(e) => {
                        println!("   effect made; play -> {:?}", e.play());
                        std::thread::sleep(Duration::from_millis(1500));
                        println!("   stop -> {:?}", e.stop());
                    }
                    Err(e) => println!("   could not make the effect: {e}"),
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        }
    }
    drop(g);

    println!("== phase B: rrt_input::Input::rumble");
    let mut input = Input::new();
    wait(&mut input, 0.5);
    for line in input.gamepads() {
        println!("  {line}");
    }
    for (label, m) in [
        ("large motor at 255", Motors { small: false, large: 255 }),
        ("small motor on", Motors { small: true, large: 0 }),
        ("both, large at 192 (the demo's Circle)", Motors { small: true, large: 0xc0 }),
    ] {
        println!("== phase B: {label}, 1.5 s");
        input.rumble(m);
        wait(&mut input, 1.5);
        input.rumble(Motors::default());
        wait(&mut input, 0.5);
    }
    println!("== done");
}
