#[cxx::bridge(namespace = "mcrl2::pbes_system")]
pub mod ffi {

    unsafe extern "C++" {
        include!("mcrl2-sys/cpp/pbes/pbes.h");

        type pbes;

        fn load_pbes_from_file(filename: &str) -> Result<UniquePtr<pbes>>;
    }
}
