/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

use bluetooth_blacklist::{Blacklist, uuid_is_blacklisted};
use dom::bindings::cell::DOMRefCell;
use dom::bindings::codegen::Bindings::BluetoothCharacteristicPropertiesBinding::
    BluetoothCharacteristicPropertiesMethods;
use dom::bindings::codegen::Bindings::BluetoothDeviceBinding::BluetoothDeviceMethods;
use dom::bindings::codegen::Bindings::BluetoothRemoteGATTCharacteristicBinding;
use dom::bindings::codegen::Bindings::BluetoothRemoteGATTCharacteristicBinding::
    BluetoothRemoteGATTCharacteristicMethods;
use dom::bindings::codegen::Bindings::BluetoothRemoteGATTServerBinding::BluetoothRemoteGATTServerMethods;
use dom::bindings::codegen::Bindings::BluetoothRemoteGATTServiceBinding::BluetoothRemoteGATTServiceMethods;
use dom::bindings::error::{ErrorResult, Fallible};
use dom::bindings::error::Error::{self, InvalidModification, Network, NotSupported, Security};
use dom::bindings::global::GlobalRef;
use dom::bindings::js::{JS, MutHeap, Root};
use dom::bindings::reflector::{Reflectable, Reflector, reflect_dom_object};
use dom::bindings::str::{ByteString, DOMString};
use dom::bluetooth::result_to_promise;
use dom::bluetoothcharacteristicproperties::BluetoothCharacteristicProperties;
use dom::bluetoothremotegattdescriptor::BluetoothRemoteGATTDescriptor;
use dom::bluetoothremotegattservice::BluetoothRemoteGATTService;
use dom::bluetoothuuid::{BluetoothDescriptorUUID, BluetoothUUID};
use dom::promise::Promise;
use ipc_channel::ipc::{self, IpcSender};
use net_traits::bluetooth_thread::BluetoothMethodMsg;
use std::rc::Rc;

// Maximum length of an attribute value.
// https://www.bluetooth.org/DocMan/handlers/DownloadDoc.ashx?doc_id=286439 (Vol. 3, page 2169)
pub const MAXIMUM_ATTRIBUTE_LENGTH: usize = 512;

// https://webbluetoothcg.github.io/web-bluetooth/#bluetoothremotegattcharacteristic
#[dom_struct]
pub struct BluetoothRemoteGATTCharacteristic {
    reflector_: Reflector,
    service: MutHeap<JS<BluetoothRemoteGATTService>>,
    uuid: DOMString,
    properties: MutHeap<JS<BluetoothCharacteristicProperties>>,
    value: DOMRefCell<Option<ByteString>>,
    instance_id: String,
}

impl BluetoothRemoteGATTCharacteristic {
    pub fn new_inherited(service: &BluetoothRemoteGATTService,
                         uuid: DOMString,
                         properties: &BluetoothCharacteristicProperties,
                         instance_id: String)
                         -> BluetoothRemoteGATTCharacteristic {
        BluetoothRemoteGATTCharacteristic {
            reflector_: Reflector::new(),
            service: MutHeap::new(service),
            uuid: uuid,
            properties: MutHeap::new(properties),
            value: DOMRefCell::new(None),
            instance_id: instance_id,
        }
    }

    pub fn new(global: GlobalRef,
               service: &BluetoothRemoteGATTService,
               uuid: DOMString,
               properties: &BluetoothCharacteristicProperties,
               instanceID: String)
               -> Root<BluetoothRemoteGATTCharacteristic> {
        reflect_dom_object(box BluetoothRemoteGATTCharacteristic::new_inherited(service,
                                                                                uuid,
                                                                                properties,
                                                                                instanceID),
                           global,
                           BluetoothRemoteGATTCharacteristicBinding::Wrap)
    }

    fn get_bluetooth_thread(&self) -> IpcSender<BluetoothMethodMsg> {
        let global_root = self.global();
        let global_ref = global_root.r();
        global_ref.as_window().bluetooth_thread()
    }

    fn get_instance_id(&self) -> String {
        self.instance_id.clone()
    }

    // https://webbluetoothcg.github.io/web-bluetooth/#dom-bluetoothremotegattcharacteristic-getdescriptor
    fn get_descriptor(&self, descriptor: BluetoothDescriptorUUID) -> Fallible<Root<BluetoothRemoteGATTDescriptor>> {
        let uuid = try!(BluetoothUUID::descriptor(descriptor)).to_string();
        if uuid_is_blacklisted(uuid.as_ref(), Blacklist::All) {
            return Err(Security)
        }
        let (sender, receiver) = ipc::channel().unwrap();
        self.get_bluetooth_thread().send(
            BluetoothMethodMsg::GetDescriptor(self.get_instance_id(), uuid, sender)).unwrap();
        let descriptor = receiver.recv().unwrap();
        match descriptor {
            Ok(descriptor) => {
                Ok(BluetoothRemoteGATTDescriptor::new(self.global().r(),
                                                      self,
                                                      DOMString::from(descriptor.uuid),
                                                      descriptor.instance_id))
            },
            Err(error) => {
                Err(Error::from(error))
            },
        }
    }

    // https://webbluetoothcg.github.io/web-bluetooth/#dom-bluetoothremotegattcharacteristic-getdescriptors
    fn get_descriptors(&self,
                       descriptor: Option<BluetoothDescriptorUUID>)
                       -> Fallible<Vec<Root<BluetoothRemoteGATTDescriptor>>> {
        let mut uuid: Option<String> = None;
        if let Some(d) = descriptor {
            uuid = Some(try!(BluetoothUUID::descriptor(d)).to_string());
            if let Some(ref uuid) = uuid {
                if uuid_is_blacklisted(uuid.as_ref(), Blacklist::All) {
                    return Err(Security)
                }
            }
        };
        let (sender, receiver) = ipc::channel().unwrap();
        self.get_bluetooth_thread().send(
            BluetoothMethodMsg::GetDescriptors(self.get_instance_id(), uuid, sender)).unwrap();
        let descriptors_vec = receiver.recv().unwrap();
        match descriptors_vec {
            Ok(descriptor_vec) => {
                Ok(descriptor_vec.into_iter()
                                 .map(|desc| BluetoothRemoteGATTDescriptor::new(self.global().r(),
                                                                                self,
                                                                                DOMString::from(desc.uuid),
                                                                                desc.instance_id))
                                 .collect())
            },
            Err(error) => {
                Err(Error::from(error))
            },
        }
    }

    // https://webbluetoothcg.github.io/web-bluetooth/#dom-bluetoothremotegattcharacteristic-readvalue
    fn read_value(&self) -> Fallible<ByteString> {
        if uuid_is_blacklisted(self.uuid.as_ref(), Blacklist::Reads) {
            return Err(Security)
        }
        let (sender, receiver) = ipc::channel().unwrap();
        if !self.Service().Device().Gatt().Connected() {
            return Err(Network)
        }
        if !self.Properties().Read() {
            return Err(NotSupported)
        }
        self.get_bluetooth_thread().send(
            BluetoothMethodMsg::ReadValue(self.get_instance_id(), sender)).unwrap();
        let result = receiver.recv().unwrap();
        let value = match result {
            Ok(val) => {
                ByteString::new(val)
            },
            Err(error) => {
                return Err(Error::from(error))
            },
        };
        *self.value.borrow_mut() = Some(value.clone());
        Ok(value)
    }

    // https://webbluetoothcg.github.io/web-bluetooth/#dom-bluetoothremotegattcharacteristic-writevalue
    fn write_value(&self, value: Vec<u8>) -> ErrorResult {
        if uuid_is_blacklisted(self.uuid.as_ref(), Blacklist::Writes) {
            return Err(Security)
        }
        if value.len() > MAXIMUM_ATTRIBUTE_LENGTH {
            return Err(InvalidModification)
        }
        if !self.Service().Device().Gatt().Connected() {
            return Err(Network)
        }

        if !(self.Properties().Write() ||
             self.Properties().WriteWithoutResponse() ||
             self.Properties().AuthenticatedSignedWrites()) {
            return Err(NotSupported)
        }
        let (sender, receiver) = ipc::channel().unwrap();
        self.get_bluetooth_thread().send(
            BluetoothMethodMsg::WriteValue(self.get_instance_id(), value, sender)).unwrap();
        let result = receiver.recv().unwrap();
        match result {
            Ok(_) => Ok(()),
            Err(error) => {
                Err(Error::from(error))
            },
        }
    }
}

impl BluetoothRemoteGATTCharacteristicMethods for BluetoothRemoteGATTCharacteristic {
    // https://webbluetoothcg.github.io/web-bluetooth/#dom-bluetoothremotegattcharacteristic-properties
    fn Properties(&self) -> Root<BluetoothCharacteristicProperties> {
        self.properties.get()
    }

    // https://webbluetoothcg.github.io/web-bluetooth/#dom-bluetoothremotegattcharacteristic-service
    fn Service(&self) -> Root<BluetoothRemoteGATTService> {
        self.service.get()
    }

    // https://webbluetoothcg.github.io/web-bluetooth/#dom-bluetoothremotegattcharacteristic-uuid
    fn Uuid(&self) -> DOMString {
        self.uuid.clone()
    }

    #[allow(unrooted_must_root)]
    // https://webbluetoothcg.github.io/web-bluetooth/#dom-bluetoothremotegattcharacteristic-getdescriptor
    fn GetDescriptor(&self, descriptor: BluetoothDescriptorUUID) -> Rc<Promise> {
        result_to_promise(self.global().r(), self.get_descriptor(descriptor))
    }

    #[allow(unrooted_must_root)]
    // https://webbluetoothcg.github.io/web-bluetooth/#dom-bluetoothremotegattcharacteristic-getdescriptors
    fn GetDescriptors(&self,
                      descriptor: Option<BluetoothDescriptorUUID>)
                      -> Rc<Promise> {
        result_to_promise(self.global().r(), self.get_descriptors(descriptor))
    }

    // https://webbluetoothcg.github.io/web-bluetooth/#dom-bluetoothremotegattcharacteristic-value
    fn GetValue(&self) -> Option<ByteString> {
        self.value.borrow().clone()
    }

    #[allow(unrooted_must_root)]
    // https://webbluetoothcg.github.io/web-bluetooth/#dom-bluetoothremotegattcharacteristic-readvalue
    fn ReadValue(&self) -> Rc<Promise> {
        result_to_promise(self.global().r(), self.read_value())
    }

    #[allow(unrooted_must_root)]
    // https://webbluetoothcg.github.io/web-bluetooth/#dom-bluetoothremotegattcharacteristic-writevalue
    fn WriteValue(&self, value: Vec<u8>) -> Rc<Promise> {
        result_to_promise(self.global().r(), self.write_value(value))
    }
}
