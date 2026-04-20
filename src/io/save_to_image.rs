use crate::io::parsed_plugins::ParsedPlugin;
use crate::land::grid_access::{GridAccessor2D, Index2D, SquareGridIterator};
use crate::land::landscape_diff::LandscapeDiff;
use crate::land::terrain_map::{Vec2, Vec3};
use crate::merge::conflict::{ConflictResolver, ConflictType};
use crate::merge::relative_terrain_map::RelativeTerrainMap;
use crate::merge::relative_to::RelativeTo;
use crate::term_style::bold_red;
use crate::LandmassDiff;
use anyhow::{anyhow, Context, Result};
use image::imageops::FilterType;
use image::{DynamicImage, ImageBuffer, Luma, Pixel, Rgb};
use log::{error, trace};
use std::collections::HashSet;
use std::fs;
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};

const DEFAULT_SCALE_FACTOR: usize = 4;

fn usize_to_u32(value: usize) -> u32 {
    u32::try_from(value).expect("image dimension exceeds u32")
}

/// Saves `img` to `file_name` after resizing by `scale_factor`.
fn save_resized_image<const T: usize, I>(
    img: I,
    file_path: &Path,
    scale_factor: usize,
) -> Result<()>
where
    DynamicImage: From<I>,
{
    let directory = file_path.parent().expect("safe");

    fs::create_dir_all(directory).with_context(|| {
        anyhow!(
            "Unable to create directory `{}` for image file {}",
            directory.to_string_lossy(),
            file_path.to_string_lossy()
        )
    })?;

    assert!(scale_factor > 0, "scale_factor must be > 0");
    DynamicImage::from(img)
        .resize_exact(
            usize_to_u32(T * scale_factor),
            usize_to_u32(T * scale_factor),
            FilterType::Nearest,
        )
        .save(file_path)
        .with_context(|| anyhow!("Unable to save image file {}", file_path.to_string_lossy()))?;

    Ok(())
}

impl<P, Container> GridAccessor2D<P> for ImageBuffer<P, Container>
where
    P: Pixel,
    Container: Deref<Target = [P::Subpixel]> + DerefMut<Target = [P::Subpixel]>,
{
    fn get(&self, coords: Index2D) -> P {
        *self.get_pixel(usize_to_u32(coords.x), usize_to_u32(coords.y))
    }

    fn get_mut(&mut self, coords: Index2D) -> &mut P {
        self.get_pixel_mut(usize_to_u32(coords.x), usize_to_u32(coords.y))
    }
}

/// Types implementing [`SaveToImage`] support a method [`SaveToImage::save_to_image`].
pub trait SaveToImage {
    /// Save an image to `file_name`.
    fn save_to_image(&self, file_path: &Path);
}

impl<const T: usize> SaveToImage for RelativeTerrainMap<Vec3<i8>, T> {
    fn save_to_image(&self, _file_path: &Path) {
        // Ignore
    }
}

impl<const T: usize> SaveToImage for RelativeTerrainMap<u16, T> {
    fn save_to_image(&self, _file_path: &Path) {
        // Ignore
    }
}

impl<const T: usize> SaveToImage for RelativeTerrainMap<Vec3<u8>, T> {
    fn save_to_image(&self, file_path: &Path) {
        let mut img = ImageBuffer::new(usize_to_u32(T), usize_to_u32(T));

        for coords in self.iter_grid() {
            let new = self.get_value(coords);
            *img.get_mut(coords) = Rgb::from([new.x, new.y, new.z]);
        }

        save_resized_image::<T, _>(img, file_path, DEFAULT_SCALE_FACTOR)
            .map_err(|e| error!("{}", bold_red(format!("{e}"))))
            .ok();
    }
}

/// Calculates the min and max values of the [`RelativeTerrainMap`].
fn calculate_min_max<U: RelativeTo, const T: usize>(map: &RelativeTerrainMap<U, T>) -> (f64, f64)
where
    f64: From<U>,
{
    let mut min_value = f64::MAX;
    let mut max_value = f64::MIN;

    for coords in map.iter_grid() {
        let value = map.get_value(coords);
        min_value = min_value.min(f64::from(value));
        max_value = max_value.max(f64::from(value));
    }

    (min_value, max_value)
}

impl<const T: usize> SaveToImage for RelativeTerrainMap<u8, T> {
    fn save_to_image(&self, file_path: &Path) {
        let mut img = ImageBuffer::new(usize_to_u32(T), usize_to_u32(T));

        let (min_value, max_value) = calculate_min_max(self);

        for coords in self.iter_grid() {
            let value = f64::from(self.get_value(coords));
            let scaled = (value - min_value) / (max_value - min_value);
            let scaled = (scaled * 255.0).clamp(0.0, 255.0);
            let as_text = format!("{scaled:.0}");
            let as_u8 = as_text.parse::<u8>().expect("scaled value within 0..=255");
            *img.get_mut(coords) = Luma::from([as_u8]);
        }

        save_resized_image::<T, _>(img, file_path, DEFAULT_SCALE_FACTOR)
            .map_err(|e| error!("{}", bold_red(format!("{e}"))))
            .ok();
    }
}

impl<const T: usize> SaveToImage for RelativeTerrainMap<i32, T> {
    fn save_to_image(&self, file_path: &Path) {
        let mut img = ImageBuffer::new(usize_to_u32(T), usize_to_u32(T));

        let (min_value, max_value) = calculate_min_max(self);

        for coords in self.iter_grid() {
            let value = f64::from(self.get_value(coords));
            let scaled = (value - min_value) / (max_value - min_value);
            let as_text = format!("{:.0}", (scaled * 255.0).clamp(0.0, 255.0));
            let as_u8 = as_text.parse::<u8>().expect("scaled value within 0..=255");
            if self.has_difference(coords) {
                let dark = format!("{:.0}", (f64::from(as_u8) * 0.98).clamp(0.0, 255.0))
                    .parse::<u8>()
                    .expect("dark value within 0..=255");
                let light = format!("{:.0}", (f64::from(as_u8) * 1.04).clamp(0.0, 255.0))
                    .parse::<u8>()
                    .expect("light value within 0..=255");
                *img.get_mut(coords) = Rgb::from([dark, light, dark]);
            } else {
                *img.get_mut(coords) = Rgb::from([as_u8, as_u8, as_u8]);
            }
        }

        save_resized_image::<T, _>(img, file_path, DEFAULT_SCALE_FACTOR)
            .map_err(|e| error!("{}", bold_red(format!("{e}"))))
            .ok();
    }
}

/// Saves an image of the conflicts between the `lhs` [`RelativeTerrainMap`] and
/// the `rhs` [`RelativeTerrainMap`] if any exist.
pub fn save_image<U: RelativeTo + ConflictResolver, const T: usize>(
    merged_lands_dir: &Path,
    coords: Vec2<i32>,
    plugin: &ParsedPlugin,
    value: &str,
    lhs: Option<&RelativeTerrainMap<U, T>>,
    rhs: Option<&RelativeTerrainMap<U, T>>,
    written_merged_images: &mut HashSet<String>,
) where
    RelativeTerrainMap<U, T>: SaveToImage,
{
    let Some(lhs) = lhs else {
        return;
    };

    let Some(rhs) = rhs else {
        return;
    };

    let mut diff_img = ImageBuffer::new(usize_to_u32(T), usize_to_u32(T));

    let mut num_major_conflicts = 0;
    let mut num_minor_conflicts = 0;

    let params = crate::merge::conflict::ConflictParams::default();

    for coords in lhs.iter_grid() {
        let actual = lhs.get_value(coords);
        let expected = rhs.get_value(coords);
        let has_difference = rhs.has_difference(coords);

        // TODO(dvd): #feature Use a gradient so that smaller conflicts can be seen.
        match actual.average(expected, &params) {
            None => {
                let color = if has_difference {
                    Rgb::from([0, 255u8, 0])
                } else {
                    Rgb::from([0, 0, 0])
                };

                *diff_img.get_mut(coords) = color;
            }
            Some(ConflictType::Minor(_)) => {
                let color = if has_difference {
                    num_minor_conflicts += 1;
                    Rgb::from([255u8, 255u8, 0])
                } else {
                    Rgb::from([0, 0, 0])
                };

                *diff_img.get_mut(coords) = color;
            }
            Some(ConflictType::Major(_)) => {
                let color = if has_difference {
                    num_major_conflicts += 1;
                    Rgb::from([255u8, 0, 0])
                } else {
                    Rgb::from([0, 0, 0])
                };

                *diff_img.get_mut(coords) = color;
            }
        }
    }

    if num_minor_conflicts == 0 && num_major_conflicts == 0 {
        return;
    }

    // TODO(dvd): #mvp Read thresholds from config.
    let num_cells = T * T;
    let minor_conflict_threshold = (num_cells / 50).max(1);
    let major_conflict_threshold = (num_cells / 1000).max(1);

    let mut should_skip = num_minor_conflicts < minor_conflict_threshold
        && num_major_conflicts < major_conflict_threshold;

    // TODO(dvd): #mvp Configure this too.
    if value == "vertex_colors" || value == "vertex_normals" {
        should_skip = true;
    }

    trace!(
        "({:>4}, {:>4}) {:<15} | {:<50} | {:>4} Major | {:>4} Minor{}",
        coords.x,
        coords.y,
        value,
        plugin.name,
        num_major_conflicts,
        num_minor_conflicts,
        if should_skip {
            String::new()
        } else {
            bold_red(" *")
        }
    );

    if should_skip {
        return;
    }

    {
        let file_name = format!(
            "{}_{}_{}_DIFF_{}.png",
            value, coords.x, coords.y, plugin.name,
        );

        let file_path: PathBuf = [
            merged_lands_dir,
            Path::new("Conflicts"),
            &PathBuf::from(file_name),
        ]
        .iter()
        .collect();

        save_resized_image::<T, _>(diff_img, &file_path, DEFAULT_SCALE_FACTOR)
            .map_err(|e| error!("{}", bold_red(format!("{e}"))))
            .ok();
    }

    {
        let file_name = format!("{}_{}_{}_MERGED.png", value, coords.x, coords.y);
        if !written_merged_images.insert(file_name.clone()) {
            return;
        }

        let file_path: PathBuf = [
            merged_lands_dir,
            Path::new("Conflicts"),
            &PathBuf::from(file_name),
        ]
        .iter()
        .collect();
        lhs.save_to_image(&file_path);
    }
}

/// Saves images of conflicts between [`LandscapeDiff`] `reference` and `plugin`.
fn save_landscape_images(
    merged_lands_dir: &Path,
    parsed_plugin: &ParsedPlugin,
    reference: &LandscapeDiff,
    plugin: &LandscapeDiff,
    written_merged_images: &mut HashSet<String>,
) {
    save_image(
        merged_lands_dir,
        reference.coords,
        parsed_plugin,
        "height_map",
        reference.height_map.as_ref(),
        plugin.height_map.as_ref(),
        written_merged_images,
    );
    save_image(
        merged_lands_dir,
        reference.coords,
        parsed_plugin,
        "vertex_normals",
        reference.vertex_normals.as_ref(),
        plugin.vertex_normals.as_ref(),
        written_merged_images,
    );
    save_image(
        merged_lands_dir,
        reference.coords,
        parsed_plugin,
        "world_map_data",
        reference.world_map_data.as_ref(),
        plugin.world_map_data.as_ref(),
        written_merged_images,
    );
    save_image(
        merged_lands_dir,
        reference.coords,
        parsed_plugin,
        "vertex_colors",
        reference.vertex_colors.as_ref(),
        plugin.vertex_colors.as_ref(),
        written_merged_images,
    );
}

/// Saves images of conflicts between [`LandmassDiff`] `reference` and all modded landmasses.
pub fn save_landmass_images(
    merged_lands_dir: &Path,
    reference: &LandmassDiff,
    modded_landmasses: &[LandmassDiff],
) {
    let mut written_merged_images = HashSet::new();

    for plugin in modded_landmasses {
        for (coords, land) in plugin.sorted() {
            let merged_land = reference.land.get(coords).expect("safe");
            save_landscape_images(
                merged_lands_dir,
                &plugin.plugin,
                merged_land,
                land,
                &mut written_merged_images,
            );
        }
    }
}
