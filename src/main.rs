mod body;
mod camera;
mod mesh;
mod mesh_renderer;
mod stl_processor;
mod texture;
use log::debug;
use mesh_renderer::MeshRenderer;
use slint::platform::PointerEventButton;
use std::num::NonZeroU32;
use std::rc::Rc;
use stl_processor::StlProcessor;
slint::include_modules!();
use body::Body;
use glow::HasContext;
use std::cell::RefCell;

macro_rules! define_scoped_binding {
    (struct $binding_ty_name:ident => $obj_name:path, $param_name:path, $binding_fn:ident, $target_name:path) => {
        struct $binding_ty_name {
            saved_value: Option<$obj_name>,
            gl: Rc<glow::Context>,
        }

        impl $binding_ty_name {
            unsafe fn new(gl: &Rc<glow::Context>, new_binding: Option<$obj_name>) -> Self {
                let saved_value =
                    NonZeroU32::new(gl.get_parameter_i32($param_name) as u32).map($obj_name);

                gl.$binding_fn($target_name, new_binding);
                Self {
                    saved_value,
                    gl: gl.clone(),
                }
            }
        }

        impl Drop for $binding_ty_name {
            fn drop(&mut self) {
                unsafe {
                    self.gl.$binding_fn($target_name, self.saved_value);
                }
            }
        }
    };
    (struct $binding_ty_name:ident => $obj_name:path, $param_name:path, $binding_fn:ident) => {
        struct $binding_ty_name {
            saved_value: Option<$obj_name>,
            gl: Rc<glow::Context>,
        }

        impl $binding_ty_name {
            unsafe fn new(gl: &Rc<glow::Context>, new_binding: Option<$obj_name>) -> Self {
                let saved_value =
                    NonZeroU32::new(gl.get_parameter_i32($param_name) as u32).map($obj_name);

                gl.$binding_fn(new_binding);
                Self {
                    saved_value,
                    gl: gl.clone(),
                }
            }
        }

        impl Drop for $binding_ty_name {
            fn drop(&mut self) {
                unsafe {
                    self.gl.$binding_fn(self.saved_value);
                }
            }
        }
    };
}

// define_scoped_binding!(struct ScopedTextureBinding => glow::NativeTexture, glow::TEXTURE_BINDING_2D, bind_texture, glow::TEXTURE_2D);
define_scoped_binding!(struct ScopedFrameBufferBinding => glow::NativeFramebuffer, glow::DRAW_FRAMEBUFFER_BINDING, bind_framebuffer, glow::DRAW_FRAMEBUFFER);
define_scoped_binding!(struct ScopedVBOBinding => glow::NativeBuffer, glow::ARRAY_BUFFER_BINDING, bind_buffer, glow::ARRAY_BUFFER);
define_scoped_binding!(struct ScopedVAOBinding => glow::NativeVertexArray, glow::VERTEX_ARRAY_BINDING, bind_vertex_array);
#[derive(Default)]
struct MouseState {
    x: f32,
    y: f32,
    p_x: f32,
    p_y: f32,
    left_pressed: bool,
    middle_pressed: bool,
    right_pressed: bool,
    other_pressed: bool,
    back_pressed: bool,
    forward_pressed: bool,
}
type SharedBodies = Rc<RefCell<Vec<Rc<Body>>>>;
type SharedMeshRenderer = Rc<RefCell<Option<MeshRenderer>>>;
type SharedMouseState = Rc<RefCell<MouseState>>;

struct AppState {
    mouse_state: SharedMouseState,
    shared_mesh_renderer: SharedMeshRenderer,
    shared_bodies: SharedBodies,
}

fn main() {
    // Initialize the Slint application
    let app = App::new().unwrap();
    let app_weak = app.as_weak();

    let state = AppState {
        mouse_state: Rc::new(RefCell::new(MouseState::default())),
        shared_mesh_renderer: Rc::new(RefCell::new(None)),
        shared_bodies: Rc::new(RefCell::new(Vec::<Rc<Body>>::new())), // Initialized as empty Vec
    };

    // let size = app.window().size();
    let interal_render_width = 1920;
    let internal_render_height = 1080;
    {
        // Set the rendering notifier with a closure
        // Create a weak reference to the app for use inside the closure
        let app_weak_clone = app_weak.clone(); // Clone app_weak for use inside the closure
        let mesh_renderer_clone = Rc::clone(&state.shared_mesh_renderer);
        if let Err(error) = app.window().set_rendering_notifier({
            // Move clones into the closure

            move |state, graphics_api| {
                match state {
                    slint::RenderingState::RenderingSetup => {
                        // Initialize OpenGL context
                        let context = match graphics_api {
                            slint::GraphicsAPI::NativeOpenGL { get_proc_address } => unsafe {
                                glow::Context::from_loader_function_cstr(|s| get_proc_address(s))
                            },
                            _ => panic!("Unsupported Graphics API"),
                        };
                        // Because the renderer needs access to the OpenGL context, we need to initialize
                        // it here instead of earlier in the state initialization
                        let renderer = MeshRenderer::new(
                            context,
                            interal_render_width,
                            internal_render_height,
                        );
                        // Store the renderer in the shared Rc<RefCell<_>>
                        *mesh_renderer_clone.borrow_mut() = Some(renderer);
                        // Initialize the borrowable bodies
                        // *bodies_clone.borrow_mut() = Some(bodies);
                    }
                    slint::RenderingState::BeforeRendering => {
                        // Access the renderer
                        if let Some(renderer) = mesh_renderer_clone.borrow_mut().as_mut() {
                            // Get actual window size
                            if let Some(app) = app_weak_clone.upgrade() {
                                // Render and get the texture
                                let texture = renderer.render(
                                    interal_render_width as u32,
                                    internal_render_height as u32,
                                );

                                // Update the app's texture
                                app.set_texture(slint::Image::from(texture));
                                app.window().request_redraw();
                            }
                        }
                    }
                    slint::RenderingState::AfterRendering => {
                        // Optional: Perform any post-rendering tasks
                    }
                    slint::RenderingState::RenderingTeardown => {
                        // Clean up the renderer
                        *mesh_renderer_clone.borrow_mut() = None;
                    }
                    _ => {}
                }
            }
        }) {
            match error {
            slint::SetRenderingNotifierError::Unsupported => eprintln!(
                "This example requires the use of the GL backend. Please run with the environment variable SLINT_BACKEND=GL set."
            ),
            _ => unreachable!(),
        }
            std::process::exit(1);
        }
    }

    // Handler for scrollwheel zooming TODO: Consider renaming for clarity
    {
        let app_weak_clone = app_weak.clone(); // Clone app_weak again for this closure
        let mesh_renderer_clone = Rc::clone(&state.shared_mesh_renderer); // Clone mesh_renderer for this closure
        app.on_zoom(move |amt| {
            // Access the renderer
            if let Some(renderer) = mesh_renderer_clone.borrow_mut().as_mut() {
                // Move the camera
                renderer.zoom(amt);

                // Trigger a redraw
                if let Some(app) = app_weak_clone.upgrade() {
                    app.window().request_redraw();
                }
            }
        });
    }

    // Handler for mouse movement in renderer
    {
        let app_weak_clone = app_weak.clone(); // Clone app_weak again for this closure
        let mesh_renderer_clone = Rc::clone(&state.shared_mesh_renderer); // Clone mesh_renderer for this closure
        let mouse_state_clone = Rc::clone(&state.mouse_state);
        app.on_mouse_move_renderer(move |x, y| {
            debug!("On mouse move event received");

            let mut mouse_state = mouse_state_clone.borrow_mut();

            // If the previous coords are still 0,0 then let's not move a bunch and return 0
            let delta_x = x - if mouse_state.p_x != 0.0 {
                mouse_state.p_x
            } else {
                x
            };
            let delta_y = y - if mouse_state.p_y != 0.0 {
                mouse_state.p_y
            } else {
                y
            };
            mouse_state.p_x = x;
            mouse_state.p_y = y;
            mouse_state.x = x;
            mouse_state.y = y;
            debug!("Delta x: {:.3}, Delta y: {:.3}", delta_x, delta_y);
            debug!("Mouse pressed? {}", mouse_state.left_pressed);

            // Access the renderer
            if let Some(renderer) = mesh_renderer_clone.borrow_mut().as_mut() {
                if mouse_state.left_pressed {
                    renderer.camera_pitch_yaw(delta_x, delta_y);
                }
                if mouse_state.middle_pressed {
                    renderer.camera_pan(delta_x, delta_y);
                }
                // Trigger a redraw
                if let Some(app) = app_weak_clone.upgrade() {
                    app.window().request_redraw();
                }
            }
        });
    }

    // Mouse down handler for renderer
    {
        let mouse_state_clone = Rc::clone(&state.mouse_state);
        app.on_mouse_down_renderer(move |button| {
            debug!("On mouse down received");
            let mut mouse_state = mouse_state_clone.borrow_mut();
            match button {
                PointerEventButton::Left => mouse_state.left_pressed = true,
                PointerEventButton::Other => mouse_state.other_pressed = true,
                PointerEventButton::Right => mouse_state.right_pressed = true,
                PointerEventButton::Middle => mouse_state.middle_pressed = true,
                PointerEventButton::Back => mouse_state.back_pressed = true,
                PointerEventButton::Forward => mouse_state.forward_pressed = true,
                _ => {}
            }
        });
    }
    // Mouse up handler for renderer
    {
        let mouse_state_clone = Rc::clone(&state.mouse_state);
        app.on_mouse_up_renderer(move |button| {
            debug!("On mouse up received");
            let mut mouse_state = mouse_state_clone.borrow_mut();
            match button {
                PointerEventButton::Left => mouse_state.left_pressed = false,
                PointerEventButton::Other => mouse_state.other_pressed = false,
                PointerEventButton::Right => mouse_state.right_pressed = false,
                PointerEventButton::Middle => mouse_state.middle_pressed = false,
                PointerEventButton::Back => mouse_state.back_pressed = false,
                PointerEventButton::Forward => mouse_state.forward_pressed = false,
                _ => {}
            }
        });
    }
    let stl_processor = StlProcessor::new();
    // Click handler for load default models button
    {
        let app_weak_clone = app_weak.clone(); // Clone app_weak again for this closure
        let mesh_renderer_clone = Rc::clone(&state.shared_mesh_renderer);
        let bodies_clone = Rc::clone(&state.shared_bodies);

        app.on_click_load_default_models(move || {
            println!("Loading default models");
            let example_stl = "ogre.stl";
            let example_stl_2 = "cube.stl";

            // Mutably borrow the Vec<Rc<Body>> and push new bodies
            {
                let mut bodies_vec = bodies_clone.borrow_mut();

                bodies_vec.push(Rc::new(Body::new_from_stl(&example_stl, &stl_processor)));
                bodies_vec.push(Rc::new(Body::new_from_stl(&example_stl_2, &stl_processor)));
            }

            // Access the renderer and add new bodies
            if let Some(renderer) = mesh_renderer_clone.borrow_mut().as_mut() {
                let bodies_vec = bodies_clone.borrow();
                for body in bodies_vec.iter() {
                    renderer.add_body(body.clone());
                }
            }

            // Trigger a redraw
            if let Some(app) = app_weak_clone.upgrade() {
                app.window().request_redraw();
            }
        });
    }
    // Run the Slint application
    app.run().unwrap();
}
