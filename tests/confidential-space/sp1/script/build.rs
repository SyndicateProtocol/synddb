use sp1_build::build_program_with_args;

fn main() {
    // SP1 program moved to crates/synddb-bootstrap/sp1/program/
    build_program_with_args(
        "../../../../crates/synddb-bootstrap/sp1/program",
        Default::default(),
    )
}
