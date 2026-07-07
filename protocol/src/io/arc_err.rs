use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct ArcErr(Arc<anyhow::Error>);
struct ArcErrForAnyhow(ArcErr);

impl ArcErr {
  pub fn from_anyhow(e: anyhow::Error) -> Self {
    ArcErr(Arc::new(e))
  }
  pub fn new(e: impl std::error::Error + 'static + Send + Sync) -> Self {
    Self::from_anyhow(anyhow::Error::new(e))
  }
  pub fn into_anyhow(self) -> anyhow::Error {
    anyhow::Error::new(ArcErrForAnyhow(self))
  }
  pub fn clone_anyhow(&self) -> anyhow::Error {
    anyhow::Error::new(ArcErrForAnyhow(self.clone()))
  }
}
impl std::fmt::Display for ArcErr {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    std::fmt::Display::fmt(&self.0, f)
  }
}
impl std::fmt::Debug for ArcErr {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    std::fmt::Debug::fmt(&self.0, f)
  }
}
impl std::fmt::Display for ArcErrForAnyhow {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    std::fmt::Display::fmt(&self.0, f)
  }
}
impl std::fmt::Debug for ArcErrForAnyhow {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    std::fmt::Debug::fmt(&self.0, f)
  }
}

impl std::error::Error for ArcErrForAnyhow {
  fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
    self.0.0.source()
  }
}
impl<E: std::error::Error + 'static + Send + Sync> From<E> for ArcErr {
  fn from(e: E) -> Self {
    ArcErr::new(e)
  }
}
impl From<ArcErr> for anyhow::Error {
  fn from(t: ArcErr) -> Self {
    t.into_anyhow()
  }
}
impl<'a> From<&'a ArcErr> for anyhow::Error {
  fn from(t: &'a ArcErr) -> Self {
    t.clone_anyhow()
  }
}
impl<'a> From<&'a mut ArcErr> for anyhow::Error {
  fn from(t: &'a mut ArcErr) -> Self {
    t.clone_anyhow()
  }
}
