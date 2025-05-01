#[macro_export]
macro_rules! nightly {
    ($printer: expr, $do: block) => {
        if cfg!(feature = "nightly") {
            $do;
        } else {
            $printer
                .println("This is a nightly feature and not yet available in the default build");
        }
    };
}
