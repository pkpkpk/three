use std::ops;
use std::sync::mpsc;

use cgmath::Ortho;
use froggy::Pointer;
use mint;

use {Object, VisualObject, LightObject, Message, Operation,
     Node, SubNode, Scene, ShadowProjection, Transform};
use factory::{Geometry, ShadowMap, Texture};


pub type Color = u32;

#[derive(Clone, Debug, PartialEq)]
pub enum Background {
    Color(Color),
    //TODO: texture, cubemap
}

#[derive(Clone, Debug)]
pub enum Material {
    LineBasic { color: Color },
    MeshBasic { color: Color, map: Option<Texture<[f32; 4]>>, wireframe: bool },
    MeshLambert { color: Color },
    MeshPhong { color: Color, glossiness: f32 },
    Sprite { map: Texture<[f32; 4]> },
}

#[derive(Clone, Debug)]
pub struct WorldNode {
    //TODO: detach from cgmath
    pub transform: Transform,
    pub visible: bool,
}

macro_rules! def_proxy {
    ($name:ident<$target:ty> = $message:ident) => {
        pub struct $name<'a> {
            value: &'a mut $target,
            node: &'a Pointer<Node>,
            tx: &'a mpsc::Sender<Message>,
        }

        impl<'a> ops::Deref for $name<'a> {
            type Target = $target;
            fn deref(&self) -> &Self::Target {
                self.value
            }
        }

        impl<'a> ops::DerefMut for $name<'a> {
            fn deref_mut(&mut self) -> &mut Self::Target {
                self.value
            }
        }

        impl<'a> Drop for $name<'a> {
            fn drop(&mut self) {
                let msg = Operation::$message(self.value.clone());
                let _ = self.tx.send((self.node.downgrade(), msg));
            }
        }
    }
}

def_proxy!(MaterialProxy<Material> = SetMaterial);

impl Object {
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
        let msg = Operation::SetVisible(visible);
        let _ = self.tx.send((self.node.downgrade(), msg));
    }

    pub fn look_at<P>(&mut self, eye: P, target: P, up: Option<mint::Vector3<f32>>)
    where P: Into<[f32; 3]>
    {
        use cgmath::{InnerSpace, Point3, Quaternion, Rotation, Vector3};
        //TEMP
        let p: [[f32; 3]; 2] = [eye.into(), target.into()];
        let dir = (Point3::from(p[0]) - Point3::from(p[1])).normalize();
        let z = Vector3::unit_z();
        let up = match up {
            Some(v) => {
                let vf: [f32; 3] = v.into();
                Vector3::from(vf).normalize()
            },
            None if dir.dot(z).abs() < 0.99 => z,
            None => Vector3::unit_y(),
        };
        let q = Quaternion::look_at(dir, up).invert();
        let qv: [f32; 3] = q.v.into();
        let rot = mint::Quaternion {
            s: q.s,
            v: qv.into(),
        };
        self.set_transform(p[0], rot, 1.0);
    }

    pub fn set_transform<P, Q>(&mut self, pos: P, rot: Q, scale: f32) where
        P: Into<mint::Point3<f32>>,
        Q: Into<mint::Quaternion<f32>>,
    {
        let msg = Operation::SetTransform(Some(pos.into()), Some(rot.into()), Some(scale));
        let _ = self.tx.send((self.node.downgrade(), msg));
    }

    pub fn set_position<P>(&mut self, pos: P) where P: Into<mint::Point3<f32>> {
        let msg = Operation::SetTransform(Some(pos.into()), None, None);
        let _ = self.tx.send((self.node.downgrade(), msg));
    }

    pub fn set_orientation<Q>(&mut self, rot: Q) where Q: Into<mint::Quaternion<f32>> {
        let msg = Operation::SetTransform(None, Some(rot.into()), None);
        let _ = self.tx.send((self.node.downgrade(), msg));
    }

    pub fn set_scale(&mut self, scale: f32) {
        let msg = Operation::SetTransform(None, None, Some(scale));
        let _ = self.tx.send((self.node.downgrade(), msg));
    }

    pub fn sync(&mut self, scene: &Scene) -> WorldNode {
        let mut hub = scene.hub.lock().unwrap();
        hub.process_messages();
        let node = &hub.nodes[&self.node];
        assert_eq!(node.scene_id, Some(scene.unique_id));
        WorldNode {
            transform: node.world_transform,
            visible: node.world_visible,
        }
    }
}

impl VisualObject {
    pub fn material(&self) -> &Material {
        &self.data.material
    }

    pub fn material_mut(&mut self) -> MaterialProxy {
        MaterialProxy {
            value: &mut self.data.material,
            node: &self.inner.node,
            tx: &self.inner.tx,
        }
    }

    pub fn sync(&mut self, scene: &Scene) -> WorldNode {
        let mut hub = scene.hub.lock().unwrap();
        hub.process_messages();
        let node = &hub.nodes[&self.node];
        assert_eq!(node.scene_id, Some(scene.unique_id));
        if let SubNode::Visual(ref data) = node.sub_node {
            self.data = data.drop_payload();
        }
        WorldNode {
            transform: node.world_transform,
            visible: node.world_visible,
        }
    }
}

impl LightObject {
    pub fn get_shadow(&self) -> Option<&ShadowMap> {
        self.data.shadow.as_ref().map(|&(ref shadow, _)| shadow)
    }
}


pub struct Group {
    object: Object,
}

impl Group {
    #[doc(hidden)]
    pub fn new(object: Object) -> Self {
        Group {
            object,
        }
    }

    pub fn add<P: AsRef<Pointer<Node>>>(&mut self, child: &P) {
        let msg = Operation::SetParent(self.object.node.clone());
        let _ = self.object.tx.send((child.as_ref().downgrade(), msg));
    }
}

pub struct Mesh {
    object: VisualObject,
    _geometry: Option<Geometry>,
}

impl Mesh {
    #[doc(hidden)]
    pub fn new(object: VisualObject) -> Self {
        Mesh {
            object,
            _geometry: None,
        }
    }
}

pub struct Sprite {
    object: VisualObject,
}

impl Sprite {
    #[doc(hidden)]
    pub fn new(object: VisualObject) -> Self {
        Sprite {
            object,
        }
    }

    pub fn set_texel_range(&mut self, base: [i16; 2], size: [u16; 2]) {
        let msg = Operation::SetTexelRange(base, size);
        let _ = self.object.tx.send((self.node.downgrade(), msg));
    }
}


pub struct AmbientLight {
    object: LightObject,
}

impl AmbientLight {
    #[doc(hidden)]
    pub fn new(object: LightObject) -> Self {
        AmbientLight {
            object,
        }
    }
}

pub struct DirectionalLight {
    object: LightObject,
    has_shadow: bool,
}

impl DirectionalLight {
    #[doc(hidden)]
    pub fn new(object: LightObject) -> Self {
        DirectionalLight {
            object,
            has_shadow: false,
        }
    }

    pub fn has_shadow(&self) -> bool {
        self.has_shadow
    }

    pub fn set_shadow(&mut self, map: ShadowMap,
                      width: f32, height: f32, near: f32, far: f32) {
        self.has_shadow = true;
        let sp = ShadowProjection::Ortho(Ortho {
            left: -0.5 * width,
            right: 0.5 * width,
            bottom: -0.5 * height,
            top: 0.5 * height,
            near,
            far,
        });
        self.data.shadow = Some((map.clone(), sp.clone()));
        let msg = Operation::SetShadow(map, sp);
        let _ = self.tx.send((self.node.downgrade(), msg));
    }
}

pub struct HemisphereLight {
    object: LightObject,
}

impl HemisphereLight {
    #[doc(hidden)]
    pub fn new(object: LightObject) -> Self {
        HemisphereLight {
            object,
        }
    }
}

pub struct PointLight {
    object: LightObject,
}

impl PointLight {
    #[doc(hidden)]
    pub fn new(object: LightObject) -> Self {
        PointLight {
            object,
        }
    }
}


impl Scene {
    pub fn add<P: AsRef<Pointer<Node>>>(&mut self, child: &P) {
        let msg = Operation::SetParent(self.node.clone());
        let _ = self.tx.send((child.as_ref().downgrade(), msg));
    }
}

macro_rules! as_node {
    ($($name:ident),*) => {
        $(
            impl AsRef<Pointer<Node>> for $name {
                fn as_ref(&self) -> &Pointer<Node> {
                    &self.object.node
                }
            }
        )*
    }
}

impl AsRef<Pointer<Node>> for LightObject {
    fn as_ref(&self) -> &Pointer<Node> {
        &self.node
    }
}

as_node!(Group, Mesh, Sprite,
         AmbientLight, DirectionalLight, HemisphereLight, PointLight);

macro_rules! deref {
    ($name:ty : $field:ident = $object:ty) => {
        impl ops::Deref for $name {
            type Target = $object;
            fn deref(&self) -> &Self::Target {
                &self.$field
            }
        }

        impl ops::DerefMut for $name {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.$field
            }
        }
    }
}

deref!(VisualObject : inner = Object);
deref!(LightObject : inner = Object);
deref!(Group : object = Object);
deref!(Mesh : object = VisualObject);
deref!(Sprite : object = VisualObject);
deref!(AmbientLight : object = LightObject);
deref!(HemisphereLight : object = LightObject);
deref!(DirectionalLight : object = LightObject);
deref!(PointLight : object = LightObject);
