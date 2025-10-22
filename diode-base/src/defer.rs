pub fn defer<F>(f: F) -> impl Drop
where
    F: FnOnce(),
{
    struct Defer<F: FnOnce()>(Option<F>);
    impl<F: FnOnce()> Drop for Defer<F> {
        fn drop(&mut self) {
            self.0.take().unwrap()();
        }
    }
    Defer(Some(f))
}

#[macro_export]
macro_rules! defer {
    ($e:expr) => {
        let _defer = $crate::defer(|| $e);
    };
    ($($data: tt)*) => {
        $crate::defer!({ $($data)* });
    };
}
