use crate::{BuiltinLoader, Format, LogLevel, PackagesHandling, Platform, Sourcemap};

#[derive(Default)]
pub struct EsbuildFlagsBuilder {
    flags: Vec<String>,
    set: u64,
}

impl EsbuildFlagsBuilder {
    const S_BUNDLE_TRUE: u64 = 1 << 0;
    const S_BUNDLE_FALSE: u64 = 1 << 1;
    const S_PACKAGES: u64 = 1 << 2;
    const S_TREE_SHAKING: u64 = 1 << 3;
    const S_PLATFORM: u64 = 1 << 4;
    const S_FORMAT: u64 = 1 << 5;

    #[inline]
    fn mark(&mut self, bit: u64) {
        self.set |= bit;
    }

    #[inline]
    fn is_set(&self, bit: u64) -> bool {
        (self.set & bit) != 0
    }

    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_defaults(mut self) -> Self {
        if !self.is_set(Self::S_FORMAT) {
            self.flags.push(format!("--format={}", Format::Esm));
        }
        if !self.is_set(Self::S_PLATFORM) {
            self.flags.push(format!("--platform={}", Platform::Node));
        }
        let bundle_is_true =
            if !self.is_set(Self::S_BUNDLE_TRUE) && !self.is_set(Self::S_BUNDLE_FALSE) {
                self.flags.push("--bundle=true".to_string());
                true
            } else {
                self.is_set(Self::S_BUNDLE_TRUE)
            };

        if bundle_is_true {
            if !self.is_set(Self::S_PACKAGES) {
                self.flags
                    .push(format!("--packages={}", PackagesHandling::Bundle));
            }
            if !self.is_set(Self::S_TREE_SHAKING) {
                self.flags.push("--tree-shaking=true".to_string());
            }
        }
        self
    }

    pub fn finish(self) -> Vec<String> {
        self.flags
    }

    pub fn build(self) -> Vec<String> {
        self.finish()
    }

    pub fn finish_with_defaults(self) -> Vec<String> {
        self.with_defaults().finish()
    }

    pub fn build_with_defaults(self) -> Vec<String> {
        self.with_defaults().build()
    }

    pub fn raw_flag(mut self, flag: impl Into<String>) -> Self {
        self.flags.push(flag.into());
        self
    }

    pub fn color(mut self, value: bool) -> Self {
        self.flags.push(format!("--color={}", value));
        self
    }

    pub fn log_level(mut self, level: LogLevel) -> Self {
        self.flags.push(format!("--log-level={}", level));
        self
    }

    pub fn log_limit(mut self, limit: u32) -> Self {
        self.flags.push(format!("--log-limit={}", limit));
        self
    }

    pub fn format(mut self, format: Format) -> Self {
        self.flags.push(format!("--format={}", format));
        self.mark(Self::S_FORMAT);
        self
    }

    pub fn platform(mut self, platform: Platform) -> Self {
        self.flags.push(format!("--platform={}", platform));
        self.mark(Self::S_PLATFORM);
        self
    }

    pub fn tree_shaking(mut self, value: bool) -> Self {
        self.flags.push(format!("--tree-shaking={}", value));
        self.mark(Self::S_TREE_SHAKING);
        self
    }

    pub fn bundle(mut self, value: bool) -> Self {
        self.flags.push(format!("--bundle={}", value));
        if value {
            self.mark(Self::S_BUNDLE_TRUE);
        } else {
            self.mark(Self::S_BUNDLE_FALSE);
        }
        self
    }

    pub fn outfile(mut self, o: impl Into<String>) -> Self {
        let o = o.into();
        self.flags.push(format!("--outfile={}", o));
        self
    }

    pub fn outdir(mut self, o: impl Into<String>) -> Self {
        let o = o.into();
        self.flags.push(format!("--outdir={}", o));
        self
    }

    pub fn packages(mut self, handling: PackagesHandling) -> Self {
        self.flags.push(format!("--packages={}", handling));
        self.mark(Self::S_PACKAGES);
        self
    }

    pub fn tsconfig(mut self, path: impl Into<String>) -> Self {
        self.flags.push(format!("--tsconfig={}", path.into()));
        self
    }

    pub fn tsconfig_raw(mut self, json: impl Into<String>) -> Self {
        self.flags.push(format!("--tsconfig-raw={}", json.into()));
        self
    }

    pub fn loader(mut self, ext: impl Into<String>, loader: BuiltinLoader) -> Self {
        self.flags
            .push(format!("--loader:{}={}", ext.into(), loader));
        self
    }

    pub fn loaders<I, K>(mut self, entries: I) -> Self
    where
        I: IntoIterator<Item = (K, BuiltinLoader)>,
        K: Into<String>,
    {
        for (k, v) in entries {
            self.flags.push(format!("--loader:{}={}", k.into(), v));
        }
        self
    }

    pub fn external(mut self, spec: impl Into<String>) -> Self {
        self.flags.push(format!("--external:{}", spec.into()));
        self
    }

    pub fn externals<I, S>(mut self, specs: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for s in specs {
            self.flags.push(format!("--external:{}", s.into()));
        }
        self
    }

    pub fn minify(mut self, value: bool) -> Self {
        if value {
            self.flags.push("--minify".to_string());
        } else {
            self.flags.push("--minify=false".to_string());
        }
        self
    }

    pub fn splitting(mut self, value: bool) -> Self {
        if value {
            self.flags.push("--splitting".to_string());
        } else {
            self.flags.push("--splitting=false".to_string());
        }
        self
    }

    pub fn define(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.flags
            .push(format!("--define:{}={}", key.into(), value.into()));
        self
    }

    pub fn defines<I, K, V>(mut self, entries: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        for (k, v) in entries {
            self.flags
                .push(format!("--define:{}={}", k.into(), v.into()));
        }
        self
    }

    pub fn metafile(mut self, value: bool) -> Self {
        if value {
            self.flags.push("--metafile".to_string());
        } else {
            self.flags.push("--metafile=false".to_string());
        }
        self
    }

    pub fn sourcemap(mut self, kind: Sourcemap) -> Self {
        self.flags.push(format!("--sourcemap={}", kind));
        self
    }

    pub fn entry_names(mut self, pattern: impl Into<String>) -> Self {
        self.flags.push(format!("--entry-names={}", pattern.into()));
        self
    }

    pub fn chunk_names(mut self, pattern: impl Into<String>) -> Self {
        self.flags.push(format!("--chunk-names={}", pattern.into()));
        self
    }

    pub fn asset_names(mut self, pattern: impl Into<String>) -> Self {
        self.flags.push(format!("--asset-names={}", pattern.into()));
        self
    }

    pub fn outbase(mut self, base: impl Into<String>) -> Self {
        self.flags.push(format!("--outbase={}", base.into()));
        self
    }

    pub fn out_extension(mut self, ext: impl Into<String>, to: impl Into<String>) -> Self {
        self.flags
            .push(format!("--out-extension:{}={}", ext.into(), to.into()));
        self
    }

    pub fn out_extensions<I, K, V>(mut self, entries: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        for (k, v) in entries {
            self.flags
                .push(format!("--out-extension:{}={}", k.into(), v.into()));
        }
        self
    }

    pub fn public_path(mut self, path: impl Into<String>) -> Self {
        self.flags.push(format!("--public-path={}", path.into()));
        self
    }

    pub fn condition(mut self, cond: impl Into<String>) -> Self {
        self.flags.push(format!("--conditions={}", cond.into()));
        self
    }

    pub fn conditions<I, S>(mut self, conds: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for c in conds {
            self.flags.push(format!("--conditions={}", c.into()));
        }
        self
    }
}
