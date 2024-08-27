use rayon::iter::{IntoParallelIterator, ParallelIterator};

use crate::{ClassInfo, ConvertedClass, DeclaredClass, ToCompiledClass};

pub trait ToConvertedClass {
    fn convert(&self) -> anyhow::Result<ConvertedClass>;
}

impl ToConvertedClass for DeclaredClass {
    fn convert(&self) -> anyhow::Result<ConvertedClass> {
        let DeclaredClass { class_hash, contract_class, compiled_class_hash } = self.clone();

        // TODO(class_hash, #212): uncomment this when the class hashes are computed correctly accross the entire state
        // let expected =
        //     contract_class.class_hash().map_err(|e| BlockImportError::ComputeClassHashError(e.to_string()))?;
        // if class_hash != expected {
        // }

        let compiled_class = contract_class.compile()?;
        // .map_err(|e| BlockImportError::CompilationClassError { error: e.to_string(), class_hash })?;

        let class_info = ClassInfo { contract_class, compiled_class_hash };

        Ok(ConvertedClass { class_infos: (class_hash, class_info), class_compiled: (class_hash, compiled_class) })
    }
}

pub trait ToConvertedClasses {
    fn convert(&self) -> anyhow::Result<Vec<ConvertedClass>>;
}

impl ToConvertedClasses for Vec<DeclaredClass> {
    fn convert(&self) -> anyhow::Result<Vec<ConvertedClass>> {
        self.into_par_iter().map(|class| class.convert()).collect()
    }
}
