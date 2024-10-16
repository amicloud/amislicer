// Distributed under the GNU Affero General Public License v3.0 or later.
// See accompanying file LICENSE or https://www.gnu.org/licenses/agpl-3.0.html for details.

use crate::body::Body;
use geo::algorithm::area::Area;
use geo::{Coord, LineString, Polygon};
use image::{ImageBuffer, Luma};
use imageproc::drawing::draw_polygon_mut;
use imageproc::point::Point;
use log::debug;
use nalgebra::{OPoint, Vector3};
use rayon::prelude::ParallelIterator;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use stl_io::{self, Triangle};

#[derive(Clone)]
pub struct BoundingBox {
    pub min: Vector3<f64>,
    pub max: Vector3<f64>,
}

#[derive(Default)]
pub struct CPUSlicer {
    pixel_x: u32,
    pixel_y: u32,
    slice_thickness: f64,
    physical_x: f64,
    physical_y: f64,
}

impl CPUSlicer {
    pub fn new(x: u32, y: u32, slice_thickness: f64, physical_x: f64, physical_y: f64) -> Self {
        CPUSlicer {
            pixel_x: x,
            pixel_y: y,
            slice_thickness,
            physical_x,
            physical_y,
        }
    }

    pub fn slice_bodies(
        &self,
        bodies: Vec<Rc<RefCell<Body>>>,
    ) -> Result<Vec<ImageBuffer<Luma<u8>, Vec<u8>>>, Box<dyn std::error::Error>> {
        let mut triangles: Vec<Triangle> = Vec::new();

        for body_rc in bodies {
            let mut body = body_rc.borrow_mut();
            body.mesh.ready_for_slicing();
            let model_matrix = body.get_model_matrix();

            for tri in &body.mesh.triangles_for_slicing {
                // Convert each vertex from [f32; 3] to OPoint<f32, 3>
                let vertex0 = OPoint::from(tri.vertices[0]);
                let vertex1 = OPoint::from(tri.vertices[1]);
                let vertex2 = OPoint::from(tri.vertices[2]);

                // Transform each vertex using the model matrix
                let transformed_vertex0 = model_matrix.transform_point(&vertex0).coords.into();
                let transformed_vertex1 = model_matrix.transform_point(&vertex1).coords.into();
                let transformed_vertex2 = model_matrix.transform_point(&vertex2).coords.into();

                let transformed_vertices = [
                    transformed_vertex0,
                    transformed_vertex1,
                    transformed_vertex2,
                ];

                // Convert normal from [f32; 3] to Vector3<f32>
                let normal_vector = Vector3::from(tri.normal);

                // Transform and normalize the normal vector
                let transformed_normal = model_matrix.transform_vector(&normal_vector).normalize();

                // Convert the transformed normal back to [f32; 3]
                let transformed_normal_array: [f32; 3] = transformed_normal.into();

                // Create a new Triangle with transformed data
                let transformed_triangle = Triangle {
                    normal: transformed_normal_array,
                    vertices: transformed_vertices,
                };

                // Add the transformed triangle to the list
                triangles.push(transformed_triangle);
            }
        }
        self.generate_slice_images(&triangles)
    }

    fn generate_slice_images(
        &self,
        triangles: &[Triangle],
    ) -> Result<Vec<ImageBuffer<Luma<u8>, Vec<u8>>>, Box<dyn std::error::Error>> {
        let (min_z, max_z) = CPUSlicer::z_range(triangles);
        let bounding_box = CPUSlicer::compute_bounding_box(triangles);
        let min_x = bounding_box.min[0];
        let max_x = bounding_box.max[0];
        let min_y = bounding_box.min[1];
        let max_y = bounding_box.max[1];
    
        let model_width = max_x - min_x;
        let model_height = max_y - min_y;
    
        // Calculate pixels per millimeter
        let ppm_x = self.pixel_x as f64 / self.physical_x;
        let ppm_y = self.pixel_y as f64 / self.physical_y;
    
        // Optionally, use the minimum ppm to maintain aspect ratio
        let ppm = ppm_x.min(ppm_y);
    
        // Update physical dimensions based on ppm to maintain aspect ratio
        let scaled_width = model_width * ppm;
        let scaled_height = model_height * ppm;
    
        // Centering offsets
        let offset_x = (self.pixel_x as f64 - scaled_width) / 2.0;
        let offset_y = (self.pixel_y as f64 - scaled_height) / 2.0;
    
        let mut slice_z_values = Vec::new();
        let mut z = min_z;
        while z <= max_z {
            slice_z_values.push(z);
            z += self.slice_thickness;
        }
    
        let images: Vec<ImageBuffer<Luma<u8>, Vec<u8>>> = slice_z_values
            .into_iter()
            .filter_map(|plane_z| {
                let segments = CPUSlicer::collect_intersection_segments(triangles, plane_z);
                if segments.is_empty() {
                    return None;
                }
    
                let polygons = CPUSlicer::assemble_polygons(&segments);
                if polygons.is_empty() {
                    return None;
                }
    
                let mut image = ImageBuffer::from_pixel(self.pixel_x, self.pixel_y, Luma([0u8]));
    
                for polygon in &polygons {
                    let mut points: Vec<Point<i32>> = Vec::new();
                    for point in polygon {
                        let (x, y) = self.model_to_image_coords(point);
                        let new_point = Point::new(x, y);
                        if !points.contains(&new_point) {
                            points.push(new_point);
                        }
                    }
    
                    // Draw the filled polygon onto the image
                    if points.len() >= 3 { // At least 3 points needed to form a polygon
                        draw_polygon_mut(&mut image, &points, Luma([255u8]));
                    }
                }
    
                Some(image)
            })
            .collect();
    
        Ok(images)
    }

    // Determine the Z-axis range of the model
    fn z_range(triangles: &[Triangle]) -> (f64, f64) {
        let z_coords: Vec<f64> = triangles
            .iter()
            .flat_map(|tri| tri.vertices.iter().map(|v| v[2] as f64))
            .collect();

        let min_z = z_coords.iter().cloned().fold(f64::INFINITY, f64::min);
        let max_z = z_coords.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

        (min_z, max_z)
    }

    // Compute the intersection of a triangle with a horizontal plane at z = plane_z
    fn intersect_triangle_with_plane(triangle: &Triangle, plane_z: f64) -> Vec<Vector3<f64>> {
        let epsilon = 1e-6; // Tolerance for floating-point comparisons

        let points: Vec<Vector3<f64>> = triangle
            .vertices
            .iter()
            .map(|v| Vector3::new(v[0] as f64, v[1] as f64, v[2] as f64))
            .collect();

        let distances: Vec<f64> = points.iter().map(|p| p[2] - plane_z).collect();

        // Check if all points are on one side of the plane
        let mut positive = false;
        let mut negative = false;

        for &distance in &distances {
            if distance > epsilon {
                positive = true;
            } else if distance < -epsilon {
                negative = true;
            }
        }

        // No intersection if all points are on one side
        if !(positive && negative) {
            return vec![];
        }

        // Find intersection points
        let mut intersections = Vec::new();

        for i in 0..3 {
            let p1 = points[i];
            let p2 = points[(i + 1) % 3];
            let d1 = distances[i];
            let d2 = distances[(i + 1) % 3];

            if (d1 > epsilon && d2 < -epsilon) || (d1 < -epsilon && d2 > epsilon) {
                let t = d1 / (d1 - d2);
                let intersection = p1 + (p2 - p1) * t;
                intersections.push(intersection);
            } else if d1.abs() <= epsilon && d2.abs() <= epsilon {
                // Both points lie on the plane
                intersections.push(p1);
                intersections.push(p2);
            } else if d1.abs() <= epsilon {
                // p1 lies on the plane
                intersections.push(p1);
            } else if d2.abs() <= epsilon {
                // p2 lies on the plane
                intersections.push(p2);
            }
        }

        // Remove duplicate points
        intersections.sort_by(|a, b| {
            a[0].partial_cmp(&b[0])
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a[1].partial_cmp(&b[1]).unwrap_or(std::cmp::Ordering::Equal))
                .then(a[2].partial_cmp(&b[2]).unwrap_or(std::cmp::Ordering::Equal))
        });
        intersections.dedup_by(|a, b| a.metric_distance(b) < epsilon);

        intersections
    }

    // Collect all intersection segments at a given plane_z
    fn collect_intersection_segments(
        triangles: &[Triangle],
        plane_z: f64,
    ) -> Vec<(Vector3<f64>, Vector3<f64>)> {
        let mut segments = Vec::new();

        for triangle in triangles {
            let intersection_points = CPUSlicer::intersect_triangle_with_plane(triangle, plane_z);

            if intersection_points.len() == 2 {
                segments.push((intersection_points[0], intersection_points[1]));
            } else if intersection_points.len() > 2 {
                debug!(
                    "Skipped a triangle intersecting the plane in multiple points at z={}",
                    plane_z
                );
            }
        }

        segments
    }

    // Assembles segments into closed polygons.
    fn assemble_polygons(segments: &[(Vector3<f64>, Vector3<f64>)]) -> Vec<Vec<Vector3<f64>>> {
        fn point_to_key(p: &Vector3<f64>, epsilon: f64) -> (i64, i64) {
            let scale = 1.0 / epsilon;
            let x = (p[0] * scale).round() as i64;
            let y = (p[1] * scale).round() as i64;
            (x, y)
        }

        let epsilon = 1e-6;
        let mut point_coords: HashMap<(i64, i64), Vector3<f64>> = HashMap::new();
        let mut adjacency: HashMap<(i64, i64), Vec<(i64, i64)>> = HashMap::new();

        // Build adjacency map
        for &(ref start, ref end) in segments {
            let start_key = point_to_key(start, epsilon);
            let end_key = point_to_key(end, epsilon);

            point_coords
                .entry(start_key)
                .or_insert_with(|| start.clone());
            point_coords.entry(end_key).or_insert_with(|| end.clone());

            adjacency.entry(start_key).or_default().push(end_key);
            adjacency.entry(end_key).or_default().push(start_key);
        }

        let mut polygons = Vec::new();
        let mut visited_edges: HashSet<((i64, i64), (i64, i64))> = HashSet::new();

        // Traverse the graph to assemble polygons
        for &start_key in adjacency.keys() {
            for &next_key in &adjacency[&start_key] {
                let edge = (start_key, next_key);
                if visited_edges.contains(&edge) || visited_edges.contains(&(next_key, start_key)) {
                    continue;
                }

                let mut polygon_keys = vec![start_key];
                let mut current_key = next_key;
                visited_edges.insert(edge);

                loop {
                    polygon_keys.push(current_key);

                    if let Some(neighbors) = adjacency.get(&current_key) {
                        // Find the next neighbor that hasn't been visited
                        let mut found = false;
                        for &neighbor_key in neighbors {
                            let edge = (current_key, neighbor_key);
                            if neighbor_key != polygon_keys[polygon_keys.len() - 2]
                                && !visited_edges.contains(&edge)
                                && !visited_edges.contains(&(neighbor_key, current_key))
                            {
                                visited_edges.insert(edge);
                                current_key = neighbor_key;
                                found = true;
                                break;
                            }
                        }

                        if !found {
                            break;
                        }

                        // Check if the polygon is closed
                        if current_key == start_key {
                            break;
                        }
                    } else {
                        break;
                    }
                }

                // Verify if we have a closed polygon
                if polygon_keys.len() >= 3 && current_key == start_key {
                    let polygon = polygon_keys
                        .into_iter()
                        .map(|key| point_coords[&key].clone())
                        .collect();
                    polygons.push(polygon);
                }
            }
        }
        polygons
    }

    #[allow(dead_code)]
    // Calculate the area of a polygon using the Shoelace formula
    fn polygon_area(polygon: &[Vector3<f64>]) -> f64 {
        let coords: Vec<Coord<f64>> = polygon.iter().map(|p| Coord { x: p[0], y: p[1] }).collect();

        let linestring = LineString::from(coords);
        let polygon = Polygon::new(linestring, vec![]);

        let area = polygon.unsigned_area();
        debug!("Polygon area: {} ", area);
        area
    }

    pub fn compute_bounding_box(triangles: &[Triangle]) -> BoundingBox {
        let mut min = Vector3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
        let mut max = Vector3::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);

        for triangle in triangles {
            for vertex in &triangle.vertices {
                min[0] = min[0].min(vertex[0] as f64);
                min[1] = min[1].min(vertex[1] as f64);
                min[2] = min[2].min(vertex[2] as f64);

                max[0] = max[0].max(vertex[0] as f64);
                max[1] = max[1].max(vertex[1] as f64);
                max[2] = max[2].max(vertex[2] as f64);
            }
        }
        BoundingBox { min, max }
    }

    // Translates points so that that 0,0 is at the center of the image
    fn model_to_image_coords(&self, model_point: &Vector3<f64>) -> (i32, i32) {
        // Calculate pixels per millimeter
        let ppm_x = self.pixel_x as f64 / self.physical_x;
        let ppm_y = self.pixel_y as f64 / self.physical_y;
    
        // Apply scaling
        let scaled_x = model_point[0] * ppm_x;
        let scaled_y = model_point[1] * ppm_y;
    
        // Translate coordinates to image space (centered)
        let image_x = scaled_x + (self.pixel_x as f64 / 2.0);
        let image_y = scaled_y + (self.pixel_y as f64 / 2.0);
    
        (image_x.round() as i32, image_y.round() as i32)
    }
}
