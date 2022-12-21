use crate::{args::ActionFile, ll};
use anni_repo::{prelude::TagRef, OwnedRepositoryManager, RepositoryManager};
use clap::{crate_version, ArgAction, Args, ValueEnum};
use clap_handler::handler;
use ptree::TreeBuilder;
use toml_edit::easy as toml;
use uuid::Uuid;

#[derive(Args, Debug, Clone)]
pub struct RepoPrintAction {
    #[clap(value_enum)]
    #[clap(short = 't', long = "type", default_value = "title")]
    #[clap(help = ll!("repo-print-type"))]
    print_type: RepoPrintType,

    #[clap(long = "no-generated-by", alias = "no-gb", action = ArgAction::SetFalse)]
    #[clap(help = ll!("repo-print-clean"))]
    add_generated_by: bool,

    #[clap(help = ll!("repo-print-input"))]
    input: String,

    #[clap(short, long, default_value = "-")]
    #[clap(help = ll!("export-to"))]
    output: ActionFile,
}

#[handler(RepoPrintAction)]
fn repo_print(me: RepoPrintAction, manager: RepositoryManager) -> anyhow::Result<()> {
    let mut dst = me.output.to_writer()?;

    match me.print_type {
        RepoPrintType::Title | RepoPrintType::Artist | RepoPrintType::Date | RepoPrintType::Cue => {
            // print album
            let split: Vec<_> = me.input.split('/').collect();
            let catalog = split[0];
            let disc_id = split
                .get(1)
                .map_or(1, |x| x.parse::<u32>().expect("Invalid disc id"));
            let disc_id = if disc_id > 0 { disc_id - 1 } else { disc_id };

            // FIXME: pick the correct album
            let mut album = manager.load_albums(catalog)?;
            let album = album.pop().unwrap();
            match me.print_type {
                RepoPrintType::Title => writeln!(dst, "{}", album.full_title())?,
                RepoPrintType::Artist => writeln!(dst, "{}", album.artist())?,
                RepoPrintType::Date => writeln!(dst, "{}", album.release_date())?,
                RepoPrintType::Cue => match album.iter().nth(disc_id as usize) {
                    Some(disc) => {
                        write!(
                            dst,
                            r#"TITLE "{title}"
PERFORMER "{artist}"
REM DATE "{date}"
"#,
                            title = disc.title(),
                            artist = disc.artist(),
                            date = album.release_date()
                        )?;
                        if me.add_generated_by {
                            write!(
                                dst,
                                r#"REM COMMENT "Generated by Anni v{}""#,
                                crate_version!()
                            )?;
                        }

                        for (track_id, track) in disc.iter().enumerate() {
                            let track_id = track_id + 1;
                            write!(
                                dst,
                                r#"
FILE "{filename}" WAVE
  TRACK 01 AUDIO
    TITLE "{title}"
    PERFORMER "{artist}"
    INDEX 01 00:00:00"#,
                                filename = format!(
                                    "{:02}. {}.flac",
                                    track_id,
                                    track.title().replace("/", "／")
                                ),
                                title = track.title(),
                                artist = track.artist(),
                            )?;
                        }
                    }
                    None => {
                        bail!("Disc {} not found!", disc_id + 1);
                    }
                },
                _ => unreachable!(),
            }
        }
        RepoPrintType::Toml | RepoPrintType::Json => {
            let text = if let Ok(album_id) = Uuid::parse_str(&me.input) {
                // me.input -> album_id
                let manager = manager.into_owned_manager()?;

                match me.print_type {
                    RepoPrintType::Toml => {
                        let root = manager.repo.root();
                        let album = manager.album_path(&album_id).expect("Album not found!");
                        let album = root.join(album);
                        anni_common::fs::read_to_string(&album)?
                    }
                    RepoPrintType::Json => {
                        let album = manager.album(&album_id).expect("Album not found!");
                        let album: serde_json::Value = album.into();
                        serde_json::to_string(&album)?
                    }
                    _ => unreachable!(),
                }
            } else {
                // me.input -> catalog
                let album = manager.load_albums(&me.input)?;
                match me.print_type {
                    RepoPrintType::Toml => toml::to_string_pretty(&album[0])?,
                    RepoPrintType::Json => {
                        let album: serde_json::Value =
                            album.get(0).expect("Album not found!").into();
                        serde_json::to_string(&album)?
                    }
                    _ => unreachable!(),
                }
            };
            write!(dst, "{text}")?;
        }
        RepoPrintType::TagTree => {
            // print tag
            let manager = manager.into_owned_manager()?;

            let tag = TagRef::from_cow_str(me.input)?;
            if manager.tag(&tag).is_none() {
                bail!("Tag not found!");
            }

            let mut tree = TreeBuilder::new(tag_to_string(&tag, &manager));
            build_tree(&manager, &tag, &mut tree);
            ptree::print_tree(&tree.build())?;

            fn tag_to_string(tag: &TagRef, manager: &OwnedRepositoryManager) -> String {
                use colored::Colorize;

                let tag_full = manager.tag(tag).unwrap();
                let tag_type = format!("[{:?}]", tag_full.tag_type()).green();
                format!("{tag_type} {}", tag_full.name())
            }

            fn build_tree(manager: &OwnedRepositoryManager, tag: &TagRef, tree: &mut TreeBuilder) {
                let child_tags = manager.child_tags(&tag);
                for tag in child_tags {
                    tree.begin_child(tag_to_string(tag, manager));
                    build_tree(manager, tag, tree);
                    tree.end_child();
                }

                if let Some(albums) = manager.albums_tagged_by(&tag) {
                    for album_id in albums {
                        let album = manager.album(album_id).unwrap();
                        tree.add_empty_child(album.full_title().to_string());
                    }
                }
            }
        }
    }

    Ok(())
}

#[derive(ValueEnum, Debug, PartialEq, Clone)]
pub enum RepoPrintType {
    Title,
    Artist,
    Date,
    Cue,
    Toml,
    Json,
    TagTree,
}
