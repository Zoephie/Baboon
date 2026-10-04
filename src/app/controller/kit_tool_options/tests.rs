use super::*;

fn layout(tags: &str, data: &str) -> KitLayout {
    KitLayout {
        root: PathBuf::from("/ek"),
        tags: PathBuf::from(tags),
        data: PathBuf::from(data),
    }
}

#[test]
fn only_folders_other_than_the_roots_own_become_options() {
    let stock = layout("/ek/tags", "/ek/data");
    let moda = layout("/ek/tags_moda", "/ek/data_moda");
    let tags_only = layout("/ek/tags_moda", "/ek/data");
    assert!(kit_tool_folder_options(&stock, Some("halo2_mcc")).is_empty());
    assert_eq!(
        kit_tool_folder_options(&moda, Some("haloce_mcc")),
        vec![
            ("-tags_dir", PathBuf::from("/ek/tags_moda")),
            ("-data_dir", PathBuf::from("/ek/data_moda")),
        ]
    );
    assert_eq!(
        kit_tool_folder_options(&tags_only, Some("halo2_mcc")),
        vec![("-tags_dir", PathBuf::from("/ek/tags_moda"))]
    );
    // The Halo 3-era tools can't take them, so they're never added there.
    assert!(kit_tool_folder_options(&moda, Some("halo3_mcc")).is_empty());
    assert!(kit_tool_folder_options(&moda, None).is_empty());
}

/// How the Microsoft C runtime splits a command line into arguments:
/// backslashes are literal unless they precede a quote, where each pair
/// is one backslash and an odd one escapes the quote; a bare quote
/// toggles quoting.
fn crt_argv(line: &str) -> Vec<String> {
    let (mut args, mut current, mut quoted, mut started) =
        (Vec::new(), String::new(), false, false);
    let chars: Vec<char> = line.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        match chars[index] {
            '\\' => {
                let run = chars[index..].iter().take_while(|&&c| c == '\\').count();
                index += run;
                if chars.get(index) == Some(&'"') {
                    current.extend(std::iter::repeat_n('\\', run / 2));
                    if run % 2 == 1 {
                        current.push('"');
                        index += 1;
                    }
                } else {
                    current.extend(std::iter::repeat_n('\\', run));
                }
                started = true;
                continue;
            }
            '"' => {
                quoted = !quoted;
                started = true;
            }
            ' ' if !quoted => {
                if started {
                    args.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            c => {
                current.push(c);
                started = true;
            }
        }
        index += 1;
    }
    if started {
        args.push(current);
    }
    args
}

/// A folder ending in a backslash, a drive root above all, reaches the
/// tool whole, and the arguments after it are still separate.
#[test]
fn a_folder_ending_in_a_backslash_reaches_the_tool_whole() {
    let options = vec![
        ("-tags_dir", PathBuf::from("D:\\")),
        ("-data_dir", PathBuf::from("E:\\kits\\data moda\\")),
    ];
    let line = with_tool_folder_options("tool bitmaps \"levels\\a\"", &options);
    assert_eq!(
        crt_argv(&line),
        [
            "tool",
            "-tags_dir",
            "D:\\",
            "-data_dir",
            "E:\\kits\\data moda\\",
            "bitmaps",
            "levels\\a",
        ],
        "{line}"
    );
    // And one that does not end in a backslash is quoted as it always was.
    assert_eq!(crt_quoted("D:\\tags"), "\"D:\\tags\"");
}

#[test]
fn options_follow_the_tool_and_nothing_else() {
    let options = vec![
        ("-tags_dir", PathBuf::from("/ek/tags moda")),
        ("-data_dir", PathBuf::from("/ek/data_moda")),
    ];
    assert_eq!(
        with_tool_folder_options("tool bitmaps \"levels\\a\"", &options),
        "tool -tags_dir \"/ek/tags moda\" -data_dir \"/ek/data_moda\" bitmaps \"levels\\a\""
    );
    assert_eq!(
        with_tool_folder_options("\"tool.exe\" model x", &options),
        "\"tool.exe\" -tags_dir \"/ek/tags moda\" -data_dir \"/ek/data_moda\" model x"
    );
    assert_eq!(
        with_tool_folder_options(".\\tool.exe", &options),
        ".\\tool.exe -tags_dir \"/ek/tags moda\" -data_dir \"/ek/data_moda\""
    );
    assert_eq!(with_tool_folder_options("dir data", &options), "dir data");
    assert_eq!(with_tool_folder_options("toolkit x", &options), "toolkit x");
    assert_eq!(with_tool_folder_options("tool x", &[]), "tool x");
}
