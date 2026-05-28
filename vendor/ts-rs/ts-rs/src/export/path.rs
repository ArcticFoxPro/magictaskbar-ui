use std::path::{Component as C, Path, PathBuf};

use super::ExportError as E;

const ERROR_MESSAGE: &str = r#"The path provided with `#[ts(export_to = "..")]` is not valid"#;

pub fn absolute<T: AsRef<Path>>(path: T) -> Result<PathBuf, E> {
    let path = std::env::current_dir()?.join(path.as_ref());

    let mut out = Vec::new();
    for comp in path.components() {
        match comp {
            C::CurDir => (),
            C::ParentDir => {
                out.pop().ok_or(E::CannotBeExported(ERROR_MESSAGE))?;
            }
            comp => out.push(comp),
        }
    }

    Ok(if !out.is_empty() {
        out.iter().collect()
    } else {
        PathBuf::from(".")
    })
}

pub(super) fn diff_paths<P, B>(path: P, base: B) -> Result<PathBuf, E>
where
    P: AsRef<Path>,
    B: AsRef<Path>,
{
    let path = absolute(path)?;
    let base = absolute(base)?;

    let mut ita = path.components();
    let mut itb = base.components();
    let mut comps: Vec<C> = vec![];

    loop {
        match (ita.next(), itb.next()) {
            (Some(C::ParentDir | C::CurDir), _) | (_, Some(C::ParentDir | C::CurDir)) => {
                unreachable!(
                    "The paths have been cleaned, no no '.' or '..' components are present"
                )
            }
            (None, None) => break,
            (Some(a), None) => {
                comps.push(a);
                comps.extend(ita.by_ref());
                break;
            }
            (None, _) => comps.push(C::ParentDir),
            (Some(a), Some(b)) if comps.is_empty() && a == b => (),
            (Some(a), Some(_)) => {
                comps.push(C::ParentDir);
                for _ in itb {
                    comps.push(C::ParentDir);
                }
                comps.push(a);
                comps.extend(ita.by_ref());
                break;
            }
        }
    }

    Ok(comps.iter().map(|c| c.as_os_str()).collect())
}
