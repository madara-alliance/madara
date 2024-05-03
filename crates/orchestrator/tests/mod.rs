#[cfg(test)]
pub mod config;

#[cfg(test)]
pub mod controllers;

#[cfg(test)]
pub mod database;

#[cfg(test)]
pub mod jobs;

#[cfg(test)]
pub mod server;

#[cfg(test)]
pub mod queue;

pub mod common;

#[cfg(test)]
pub mod tests {
    use mockall::*;
    use mockall::predicate::*;
    use starknet_core::types::NonceUpdate;
    
    #[automock]
    trait MyTrait {
        fn foo(&self) -> u32;
        fn bar(&self, x: u32, y: u32) -> u32;
    }

    struct NonClone();
    #[automock]
    trait Food {
        fn food(&self) -> NonClone;
    }

    #[test]
    fn test_this() {
        let mut mock = MockMyTrait::new();

        mock.expect_foo().times(1).return_const(42u32);

        mock.expect_bar().times(1).with(predicate::eq(3), predicate::eq(4))
        .returning(|x, y| {x+y});

        let mut mock = MockFood::new();
        let r = NonClone{};
        mock.expect_food().return_once(move || r);
    }
}