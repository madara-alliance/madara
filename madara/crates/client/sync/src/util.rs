use std::fmt;

pub fn fmt_option(opt: Option<impl fmt::Display>, or_else: impl fmt::Display) -> impl fmt::Display {
    DisplayFromFn(move |f| if let Some(val) = &opt { val.fmt(f) } else { or_else.fmt(f) })
}

pub struct DisplayFromFn<F: Fn(&mut fmt::Formatter<'_>) -> fmt::Result>(pub F);
impl<F: Fn(&mut fmt::Formatter<'_>) -> fmt::Result> fmt::Display for DisplayFromFn<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (self.0)(f)
    }
}

pub struct ServiceStateSender<T>(Option<tokio::sync::mpsc::UnboundedSender<T>>);

impl<T> Default for ServiceStateSender<T> {
    fn default() -> Self {
        Self(None)
    }
}

impl<T> ServiceStateSender<T> {
    pub fn send(&self, val: T) {
        if let Some(sender) = &self.0 {
            let _res = sender.send(val);
        }
    }
}

#[cfg(test)]
pub fn service_state_channel<T>() -> (ServiceStateSender<T>, tokio::sync::mpsc::UnboundedReceiver<T>) {
    let (sender, recv) = tokio::sync::mpsc::unbounded_channel();
    (ServiceStateSender(Some(sender)), recv)
}
