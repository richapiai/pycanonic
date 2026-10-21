use pyo3::prelude::*;

#[pymodule]
mod _core {
    use pyo3::prelude::*;

    #[pyfunction]
    fn version() -> &'static str {
        canonic_core::version()
    }
}
