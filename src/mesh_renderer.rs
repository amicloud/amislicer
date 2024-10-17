// Distributed under the GNU Affero General Public License v3.0 or later.
// See accompanying file LICENSE or https://www.gnu.org/licenses/agpl-3.0.html for details.
use std::cell::RefCell;
use std::fs;
use std::rc::Rc;
slint::include_modules!();
use crate::body::Body;
use crate::camera::Camera;
use crate::mesh::Mesh;
use crate::mesh::Vertex;
use crate::texture::Texture;
use crate::ScopedVAOBinding;
use crate::ScopedVBOBinding;
use glow::Context as GlowContext;
use glow::HasContext;
use nalgebra::Vector;
use nalgebra::Vector3;
pub struct MeshRenderer {
    gl: Rc<GlowContext>,
    program: glow::Program,
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    ebo: glow::Buffer,
    view_proj_location: glow::UniformLocation,
    view_direction_location: glow::UniformLocation,
    light_direction_location: glow::UniformLocation,
    model_location: glow::UniformLocation,
    displayed_texture: Texture,
    next_texture: Texture,
    bodies: Vec<Rc<RefCell<Body>>>,
    camera: Camera,
}

impl MeshRenderer {
    pub fn new(gl: Rc<GlowContext>, width: u32, height: u32) -> Self {
        unsafe {
            // Create shader program
            let shader_program = gl.create_program().expect("Cannot create program");
            let aspect_ratio = width as f32 / height as f32;
            let camera = Camera::new(aspect_ratio);
            let manifest_dir = env!("CARGO_MANIFEST_DIR");
            let vertex_shader_path = format!("{}/shaders/vertex_shader.glsl", manifest_dir);
            let fragment_shader_path = format!("{}/shaders/fragment_shader.glsl", manifest_dir);

            let vertex_shader_source =
                fs::read_to_string(&vertex_shader_path).expect("Failed to read vertex shader file");
            let fragment_shader_source = fs::read_to_string(&fragment_shader_path)
                .expect("Failed to read fragment shader file");

            // Compile shaders and link program
            let shader_sources = [
                (glow::VERTEX_SHADER, vertex_shader_source),
                (glow::FRAGMENT_SHADER, fragment_shader_source),
            ];

            let mut shaders = Vec::with_capacity(shader_sources.len());

            for (shader_type, shader_source) in &shader_sources {
                let shader = gl
                    .create_shader(*shader_type)
                    .expect("Cannot create shader");
                gl.shader_source(shader, shader_source);
                gl.compile_shader(shader);
                if !gl.get_shader_compile_status(shader) {
                    panic!(
                        "Fatal Error: Shader compile error: {}",
                        gl.get_shader_info_log(shader)
                    );
                }
                gl.attach_shader(shader_program, shader);
                shaders.push(shader);
            }

            gl.link_program(shader_program);
            if !gl.get_program_link_status(shader_program) {
                panic!(
                    "Fatal Error: Shader program link error: {}",
                    gl.get_program_info_log(shader_program)
                );
            }

            for shader in shaders {
                gl.detach_shader(shader_program, shader);
                gl.delete_shader(shader);
            }

            // Get attribute and uniform locations
            let view_proj_location = gl
                .get_uniform_location(shader_program, "view_proj")
                .unwrap();
            let position_location =
                gl.get_attrib_location(shader_program, "position").unwrap() as u32;
            let normal_location = gl.get_attrib_location(shader_program, "normal").unwrap() as u32;
            let view_direction_location = gl
                .get_uniform_location(shader_program, "view_direction")
                .unwrap();
            // Get attribute and uniform locations
            let light_direction_location = gl
                .get_uniform_location(shader_program, "light_direction")
                .unwrap();

            // Get attribute and uniform locations
            let model_location = gl.get_uniform_location(shader_program, "model").unwrap();

            // Set up VBO, EBO, VAO
            let vbo = gl.create_buffer().expect("Cannot create buffer");
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));

            let vao = gl
                .create_vertex_array()
                .expect("Cannot create vertex array");
            gl.bind_vertex_array(Some(vao));

            let ebo = gl.create_buffer().expect("Cannot create EBO");
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(ebo));

            // Position attribute
            gl.enable_vertex_attrib_array(position_location);
            gl.vertex_attrib_pointer_f32(
                position_location,
                3,           // size
                glow::FLOAT, // type
                false,       // normalized
                6 * 4,       // stride (6 floats per vertex)
                0,           // offset
            );

            // Normal attribute
            gl.enable_vertex_attrib_array(normal_location);
            gl.vertex_attrib_pointer_f32(
                normal_location,
                3,
                glow::FLOAT,
                true,
                6 * 4, // stride (6 floats per vertex)
                3 * 4, // offset (after the first 3 floats)
            );

            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            gl.bind_vertex_array(None);
            let width = 1920;
            let height = 1080;

            gl.enable(glow::MULTISAMPLE);
            let depth_buffer = gl.create_renderbuffer().unwrap();
            gl.bind_renderbuffer(glow::RENDERBUFFER, Some(depth_buffer));
            gl.renderbuffer_storage_multisample(
                glow::RENDERBUFFER,
                4,
                glow::DEPTH_COMPONENT16,
                width as i32,
                height as i32,
            );
            gl.framebuffer_renderbuffer(
                glow::FRAMEBUFFER,
                glow::DEPTH_ATTACHMENT,
                glow::RENDERBUFFER,
                Some(depth_buffer),
            );
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LESS);

            // Initialize textures
            let displayed_texture = Texture::new(&gl, width, height);
            let next_texture = Texture::new(&gl, width, height);
            let meshes = Vec::new();
            let mut me = Self {
                gl,
                program: shader_program,
                view_proj_location,
                view_direction_location,
                light_direction_location,
                model_location,
                vao,
                vbo,
                ebo,
                displayed_texture,
                next_texture,
                bodies: meshes,
                camera,
            };
            me.add_xy_plane(100.0);
            me
        }
    }

    pub fn render(&mut self, width: u32, height: u32) -> slint::Image {
        unsafe {
            let gl = &self.gl;
            gl.use_program(Some(self.program));
            let _saved_vbo = ScopedVBOBinding::new(gl, Some(self.vbo));
            let _saved_vao = ScopedVAOBinding::new(gl, Some(self.vao));
            // Enable face culling
            gl.disable(glow::CULL_FACE);
            gl.cull_face(glow::BACK);

            // Resize texture if necessary
            if self.next_texture.width != width || self.next_texture.height != height {
                let mut new_texture = Texture::new(gl, width, height);
                std::mem::swap(&mut self.next_texture, &mut new_texture);
            }

            self.next_texture.with_texture_as_active_fbo(|| {
                if gl.check_framebuffer_status(glow::FRAMEBUFFER) != glow::FRAMEBUFFER_COMPLETE {
                    panic!("Framebuffer is not complete!");
                }
                // **Enable depth testing inside the framebuffer binding**
                gl.enable(glow::DEPTH_TEST);
                gl.depth_func(glow::LEQUAL);
                // Clear color and depth buffers
                gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
                // Enable multisampling
                gl.enable(glow::MULTISAMPLE);
                // Save and set viewport
                let mut saved_viewport: [i32; 4] = [0; 4];
                gl.get_parameter_i32_slice(glow::VIEWPORT, &mut saved_viewport);
                gl.viewport(
                    0,
                    0,
                    self.next_texture.width as i32,
                    self.next_texture.height as i32,
                );

                // Compute view and projection matrices
                let projection = self.camera.projection_matrix;
                let view = self.camera.view_matrix();
                let view_proj = projection * view;
                // Assuming `view_proj_location`, `view_direction_location`, and `light_direction_location` are obtained using `gl.get_uniform_location` or equivalent

                let view_dir = self.camera.get_view_direction_vector();
                gl.uniform_3_f32(
                    Some(&self.view_direction_location),
                    view_dir.x,
                    view_dir.y,
                    view_dir.z,
                );

                // Set the light direction (e.g., a fixed directional light)
                gl.uniform_3_f32(Some(&self.light_direction_location), 1.0, -1.0, 0.5);

                // Convert to column-major array
                let view_proj_matrix: [f32; 16] = view_proj
                    .as_slice()
                    .try_into()
                    .expect("Slice with incorrect length");

                // Set the view_proj uniform
                gl.uniform_matrix_4_f32_slice(
                    Some(&self.view_proj_location),
                    false,
                    &view_proj_matrix,
                );

                let mut offset: i32 = 0;

                for body in &self.bodies {
                    let mesh = &body.borrow().mesh;
                    // Set the model uniform
                    gl.uniform_matrix_4_f32_slice(
                        Some(&self.model_location),
                        false,
                        &body.borrow().get_model_matrix().as_slice(),
                    );

                    // Upload the vertex data to the GPU
                    self.gl.buffer_data_u8_slice(
                        glow::ARRAY_BUFFER,
                        bytemuck::cast_slice(&mesh.vertices),
                        glow::STATIC_DRAW, // Use DYNAMIC_DRAW if you plan to update frequently
                    );

                    // Upload the index data to the GPU
                    self.gl.buffer_data_u8_slice(
                        glow::ELEMENT_ARRAY_BUFFER,
                        bytemuck::cast_slice(&mesh.indices),
                        glow::STATIC_DRAW,
                    );

                    // Unbind the buffers

                    if gl.check_framebuffer_status(glow::FRAMEBUFFER) != glow::FRAMEBUFFER_COMPLETE
                    {
                        panic!("Framebuffer is not complete!");
                    }

                    // Bind VAO and draw
                    gl.bind_vertex_array(Some(self.vao));
                    // Bind the VBO
                    self.gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
                    // Bind the EBO
                    self.gl
                        .bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(self.ebo));
                    gl.draw_elements(
                        glow::TRIANGLES,
                        mesh.indices.len() as i32, // Number of indices
                        glow::UNSIGNED_INT,
                        offset, // Offset into the EBO
                    );
                    offset += (mesh.indices.len() * 3*4) as i32;
                    gl.bind_vertex_array(None);
                    self.gl.bind_buffer(glow::ARRAY_BUFFER, None);
                    self.gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, None);
                }

                // Restore viewport
                gl.viewport(
                    saved_viewport[0],
                    saved_viewport[1],
                    saved_viewport[2],
                    saved_viewport[3],
                );
            });

            gl.use_program(None);
        }

        // Create the result texture
        let result_texture = unsafe {
            slint::BorrowedOpenGLTextureBuilder::new_gl_2d_rgba_texture(
                self.next_texture.texture.0,
                (self.next_texture.width, self.next_texture.height).into(),
            )
            .build()
        };

        // Swap textures for the next frame
        std::mem::swap(&mut self.next_texture, &mut self.displayed_texture);

        result_texture
    }

    pub fn camera_pitch_yaw(&mut self, delta_x: f32, delta_y: f32) {
        self.camera.pitch_yaw(delta_x, -delta_y);
    }

    pub fn camera_pan(&mut self, delta_x: f32, delta_y: f32) {
        self.camera.pan(delta_x, delta_y);
    }

    pub fn add_body(&mut self, body: Rc<RefCell<Body>>) {
        self.bodies.push(Rc::clone(&body)); // Clone the Rc to store a reference
    }

    pub fn remove_body(&mut self, body: Rc<RefCell<Body>>) {
        if let Some(pos) = self.bodies.iter().position(|x| Rc::ptr_eq(x, &body)) {
            self.bodies.remove(pos);
        }
    }

    pub(crate) fn zoom(&mut self, amt: f32) {
        self.camera.zoom(amt);
    }

    fn create_xy_plane_mesh(size: f32) -> Mesh {
        let vertices = vec![
            Vertex {
                position: [-size, -size, 0.0],
                normal: [0.0, 0.0, 1.0],
            },
            Vertex {
                position: [size, -size, 0.0],
                normal: [0.0, 0.0, 1.0],
            },
            Vertex {
                position: [size, size, 0.0],
                normal: [0.0, 0.0, 1.0],
            },
            Vertex {
                position: [-size, size, 0.0],
                normal: [0.0, 0.0, 1.0],
            },
        ];

        let indices = vec![
            [0, 1, 2], // First triangle
            [0, 2, 3], // Second triangle
        ];

        Mesh {
            vertices,
            indices,
            original_triangles: Vec::new(),
            triangles_for_slicing: Vec::new(),
        }
    }

    fn create_plane_body(size: f32) -> Rc<RefCell<Body>> {
        let plane_mesh = Self::create_xy_plane_mesh(size);
        let mut body = Body::new(plane_mesh);
        body.set_position(Vector3::new(0.0, 0.0, 0.0)); // Ensure the plane is at the origin
        Rc::new(RefCell::new(body))
    }

    pub fn add_xy_plane(&mut self, size: f32) {
        let plane_body = Self::create_plane_body(size);
        self.add_body(plane_body);
    }
}

impl Drop for MeshRenderer {
    fn drop(&mut self) {
        unsafe {
            self.gl.delete_program(self.program);
            self.gl.delete_vertex_array(self.vao);
            self.gl.delete_buffer(self.vbo);
        }
    }
}
