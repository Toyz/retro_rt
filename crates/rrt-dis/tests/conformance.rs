//! Both MIPS backends against the rules every backend must follow
//! (`rrt_dis::conformance`), in pseudo-op and canonical form.

use rrt_dis::conformance::check;
use rrt_dis::mips::Mips;

#[test]
fn the_mips_backends_conform() {
    for arch in [Mips::r3000(), Mips::r5900(), Mips::r3000().canonical(), Mips::r5900().canonical()] {
        let failures = check(&arch, 200_000, 0x5eed_1234);
        let shown: Vec<String> = failures.iter().take(20).map(|f| f.to_string()).collect();
        assert!(failures.is_empty(), "{} failures for {:?}:\n{}", failures.len(), arch, shown.join("\n"));
    }
}
