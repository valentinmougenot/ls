use std::path::PathBuf;

pub struct Config {
    pub format: Format,
    pub hidden_mode: HiddenMode,
    pub reverse: bool,
    pub list_dir: bool,
    pub recursive: bool,
    pub sort: Sort,
    pub human_readable: bool,
    pub paths: Vec<PathBuf>,
}

#[derive(PartialEq, Eq)]
pub enum Format {
    OneLine,
    Long,
    Default,
}

#[derive(PartialEq, Eq)]
pub enum HiddenMode {
    All,
    AlmostAll,
    Default,
}

#[derive(PartialEq, Eq)]
pub enum Sort {
    Time,
    Size,
    Default,
}
