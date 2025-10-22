use diode::AppBuilder;

pub trait BundleExt {
    fn add_bundle<F>(&mut self, func: F) -> &mut Self
    where
        F: FnOnce(&mut Self),
        Self: Sized;
}

impl BundleExt for AppBuilder {
    fn add_bundle<F>(&mut self, func: F) -> &mut Self
    where
        F: FnOnce(&mut Self),
        Self: Sized,
    {
        func(self);
        self
    }
}
