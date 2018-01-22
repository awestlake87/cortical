use std;
use std::fmt::Debug;
use std::hash::Hash;

use futures::prelude::*;
use futures::unsync;
use tokio_core::reactor;

use super::{Error, Result};

/// trait alias to express requirements of a Role type
pub trait Role: Debug + Copy + Clone + Hash + PartialEq + Eq {}

impl<T> Role for T
where
    T: Debug + Copy + Clone + Hash + PartialEq + Eq,
{
}

/// trait alias to express requirements of a Synapse type
pub trait Synapse {}

impl<T> Synapse for T {}

/// a group of control signals passed between somas
pub enum Impulse<R: Role, S: Synapse> {
    /// add an input synapse with the given role to the soma
    ///
    /// you should always expect to handle this impulse if the soma has any
    /// inputs. if your soma has inputs, it is best to wrap it with an Axon
    /// which can be used for validation purposes.
    AddInput(R, S),
    /// add an output synapse with the given role to the soma
    ///
    /// you should always expect to handle this impulse if the soma has any
    /// outputs. if your soma has outputs, it is best to wrap it with an Axon
    /// which can be used for validation purposes.
    AddOutput(R, S),
    /// notify the soma that it has received all of its inputs and outputs
    ///
    /// you should always expect to handle this impulse because it will be
    /// passed to each soma regardless of configuration
    Start(unsync::mpsc::Sender<Impulse<R, S>>, reactor::Handle),
    /// stop the event loop and exit gracefully
    ///
    /// you should not expect to handle this impulse at any time, it is handled
    /// for you by the event loop
    Stop,
    /// terminate the event loop with an error
    ///
    /// this impulse will automatically be triggered if a soma update resolves
    /// with an error.
    ///
    /// you should not expect to handle this impulse at any time, it is handled
    /// for you by the event loop
    Error(Error),
}

impl<R, S> Impulse<R, S>
where
    R: Role,
    S: Synapse,
{
    /// convert from another type of impulse
    pub fn convert_from<T, U>(imp: Impulse<T, U>) -> Self
    where
        T: Role + Into<R>,
        U: Synapse + Into<S>,
    {
        match imp {
            Impulse::AddInput(role, synapse) => {
                Impulse::AddInput(role.into(), synapse.into())
            },
            Impulse::AddOutput(role, synapse) => {
                Impulse::AddOutput(role.into(), synapse.into())
            },
            Impulse::Stop => Impulse::Stop,
            Impulse::Error(e) => Impulse::Error(e),

            Impulse::Start(_, _) => panic!("no automatic conversion for start"),
        }
    }
}

/// a singular cell of functionality that can be ported between organelles
///
/// you can think of a soma as a stream of impulses folded over a structure.
/// somas will perform some type of update upon receiving an impulse, which can
/// then propagate to other somas. when stitched together inside an organelle,
/// this can essentially be used to easily solve any asynchronous programming
/// problem in an efficient, modular, and scalable way.
pub trait Soma: Sized {
    /// the role a synapse plays in a connection between somas.
    type Role: Role + Into<(Self::Synapse, Self::Synapse)>;
    /// the glue that binds somas together.
    ///
    /// this will (probably) be an enum representing the different types of
    /// connections that can be made between this soma and others. synapses can
    /// be used to exchange synchronization primitives such as (but not limited
    /// to) mpsc and oneshot channels. these can provide custom-tailored methods
    /// of communication between somas to ease the pain of async programming.
    type Synapse: Synapse;
    /// the types of errors that this soma can return
    type Error: std::error::Error + Send + Into<Error>;
    /// the future representing a single update of the soma.
    type Future: Future<Item = Self, Error = Self::Error>;

    /// react to a single impulse
    fn update(self, imp: Impulse<Self::Role, Self::Synapse>) -> Self::Future;

    /// convert this soma into a future that can be passed to an event loop
    #[async(boxed)]
    fn run(mut self, handle: reactor::Handle) -> Result<()>
    where
        Self: 'static,
    {
        // it's important that tx live through this function
        let (tx, rx) = unsync::mpsc::channel(1);

        await!(
            tx.clone()
                .send(Impulse::Start(tx, handle))
                .map_err(|_| Error::from("unable to send start signal"))
        )?;

        #[async]
        for imp in rx.map_err(|_| Error::from("streams can't fail")) {
            match imp {
                Impulse::Error(e) => bail!(e),
                Impulse::Stop => break,

                _ => self = await!(self.update(imp)).map_err(|e| e.into())?,
            }
        }

        Ok(())
    }
}
