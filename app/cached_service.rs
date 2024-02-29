use chrono::NaiveDate;
use log::trace;

pub use crate::cache::PersonCao;
pub use crate::domain::{Person, PersonId};
pub use crate::service::{PersonService, ServiceError};

pub trait PersonCachedService<'a, Conn, Ctx>: PersonService<'a, Ctx> {
    type C: PersonCao<Conn>;

    fn get_cao(&self) -> Self::C;

    fn cached_register(
        &'a mut self,
        name: &str,
        birth_date: NaiveDate,
        death_date: Option<NaiveDate>,
        data: &str,
    ) -> Result<(PersonId, Person), ServiceError> {
        trace!(
            "cached register: {} {} {:?} {}",
            name,
            birth_date,
            death_date,
            data
        );
        let cao = self.get_cao();

        let result = self.register(name, birth_date, death_date, data);
        trace!("register person to db: {:?}", result);

        if let Ok((id, person)) = &result {
            let _: () = cao
                .run_tx(cao.load(*id, person))
                .map_err(|e| ServiceError::ServiceUnavailable(e.to_string()))?;

            trace!("load person to cache: {}", person);
        }

        result
    }

    fn cached_find(&'a mut self, id: PersonId) -> Result<Option<Person>, ServiceError> {
        trace!("cached find: {}", id);
        let cao = self.get_cao();

        // if the person is found in the cache, return it
        if let Some(p) = cao
            .run_tx(cao.find(id))
            .map_err(|e| ServiceError::ServiceUnavailable(e.to_string()))?
        {
            trace!("cache hit!: {}", id);
            return Ok(Some(p));
        }
        trace!("cache miss!: {}", id);

        let result = self.find(id)?;
        trace!("find person in db: {:?}", result);

        // if the person is found in the db, load it to the cache
        if let Some(person) = &result {
            let _: () = cao
                .run_tx(cao.load(id, person))
                .map_err(|e| ServiceError::ServiceUnavailable(e.to_string()))?;
            trace!("load person to cache: {}", person);
        }

        Ok(result)
    }

    fn cached_batch_import(
        &'a mut self,
        persons: Vec<Person>,
    ) -> Result<Vec<PersonId>, ServiceError> {
        trace!("cached batch import: {:?}", persons);
        let cao = self.get_cao();

        let ids = self.batch_import(persons.clone())?;

        // load all persons to the cache
        ids.iter().zip(persons.iter()).for_each(|(id, person)| {
            let _: () = cao.run_tx(cao.load(*id, person)).expect("load cache");
        });
        trace!("load persons to cache: {:?}", ids);

        Ok(ids)
    }

    fn cached_list_all(&'a mut self) -> Result<Vec<(PersonId, Person)>, ServiceError> {
        trace!("cached list all");
        let cao = self.get_cao();

        let result = self.list_all()?;

        // load all persons to the cache
        result.iter().for_each(|(id, person)| {
            let _: () = cao.run_tx(cao.load(*id, person)).expect("load cache");
        });
        trace!("load all persons to cache");

        Ok(result)
    }

    fn cached_unregister(&'a mut self, id: PersonId) -> Result<(), ServiceError> {
        trace!("cached unregister: {}", id);
        let cao = self.get_cao();

        // even if delete from db failed below, this cache clear is not a matter.
        let _: () = cao
            .run_tx(cao.unload(id))
            .map_err(|e| ServiceError::ServiceUnavailable(e.to_string()))?;
        trace!("unload from cache: {}", id);

        let result = self.unregister(id);
        trace!("delete from db: {}", id);

        result
    }
}

// # フェイクテスト
//
// * 目的
//
//   CachedService の正常系のテストを行う
//   CachedService の各メソッドが、 Cache と Service とから通常期待される結果を受け取ったときに
//   適切にふるまうことを保障する
//
// * 方針
//
//   Cache のフェイクと Service のフェイクに対して CachedService を実行し、その結果を確認する
//   フェイクはテスト時の比較チェックのしやすさを考慮して HashMap ではなく Vec で登録データを保持する
//   データ数は多くないので、Vec でリニアサーチしても十分な速度が出ると考える
//
// * 実装
//
//   1. ダミーの DAO 構造体、ユースケース構造体を用意する
//      この構造体は実質使われないが、 Service に必要なので用意する
//   2. CachedService のメソッド呼び出しに対して、期待される結果を返す Service の実装を用意する
//      この Service 実装はフェイクなので、間接的な入力と間接的な出力が整合するようにする
//   3. CachedService のメソッド呼び出しに対して、期待される結果を返す Cache 構造体を用意する
//      この Cache 構造体はフェイクなので、間接的な入力と間接的な出力が整合するようにする
//   4. CachedService をここまでに用意したフェイクとダミーで構築する
//   5. Service のメソッドを呼び出す
//   6. Service からの戻り値を検証する
//
// * 注意
//
//   1. このテストは CachedService の実装を保障するものであって、Service や Cache の実装を保障するものではない
//   2. 同様にこのテストは ユースケースや DAO の実装を保障するものではない
//
#[cfg(test)]
mod fake_tests {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    use crate::{
        dao::{DaoError, PersonDao},
        date, HavePersonDao, PersonUsecase, UsecaseError,
    };

    use super::*;

    struct DummyPersonDao;
    impl PersonDao<()> for DummyPersonDao {
        fn insert(&self, _person: Person) -> impl tx_rs::Tx<(), Item = PersonId, Err = DaoError> {
            tx_rs::with_tx(move |&mut ()| Ok(1))
        }
        fn fetch(
            &self,
            _id: PersonId,
        ) -> impl tx_rs::Tx<(), Item = Option<Person>, Err = DaoError> {
            tx_rs::with_tx(move |&mut ()| Ok(None))
        }
        fn select(&self) -> impl tx_rs::Tx<(), Item = Vec<(PersonId, Person)>, Err = DaoError> {
            tx_rs::with_tx(move |&mut ()| Ok(vec![]))
        }
        fn delete(&self, _id: PersonId) -> impl tx_rs::Tx<(), Item = (), Err = DaoError> {
            tx_rs::with_tx(move |&mut ()| Ok(()))
        }
    }

    struct DummyPersonUsecase {
        dao: DummyPersonDao,
    }
    impl HavePersonDao<()> for DummyPersonUsecase {
        fn get_dao<'b>(&'b self) -> Box<&impl PersonDao<()>> {
            Box::new(&self.dao)
        }
    }
    impl PersonUsecase<()> for DummyPersonUsecase {
        fn entry<'a>(
            &'a mut self,
            _person: Person,
        ) -> impl tx_rs::Tx<(), Item = PersonId, Err = UsecaseError>
        where
            (): 'a,
        {
            tx_rs::with_tx(move |&mut ()| Ok(1))
        }
        fn find<'a>(
            &'a mut self,
            _id: PersonId,
        ) -> impl tx_rs::Tx<(), Item = Option<Person>, Err = UsecaseError>
        where
            (): 'a,
        {
            tx_rs::with_tx(move |&mut ()| Ok(None))
        }
        fn entry_and_verify<'a>(
            &'a mut self,
            person: Person,
        ) -> impl tx_rs::Tx<(), Item = (PersonId, Person), Err = UsecaseError>
        where
            (): 'a,
        {
            tx_rs::with_tx(move |&mut ()| Ok((1, person)))
        }
        fn collect<'a>(
            &'a mut self,
        ) -> impl tx_rs::Tx<(), Item = Vec<(PersonId, Person)>, Err = UsecaseError>
        where
            (): 'a,
        {
            tx_rs::with_tx(move |&mut ()| Ok(vec![]))
        }
        fn remove<'a>(
            &'a mut self,
            _id: PersonId,
        ) -> impl tx_rs::Tx<(), Item = (), Err = UsecaseError>
        where
            (): 'a,
        {
            tx_rs::with_tx(move |&mut ()| Ok(()))
        }
    }

    /// テスト用のフェイクサービスです。
    /// Clone できるようにしていないので基本は Rc でラップしていません。
    /// FakePersonCao のみ get_cao() で clone されるため内部データを Rc でラップしています。
    struct TargetPersonService {
        next_id: RefCell<PersonId>,
        db: RefCell<HashMap<PersonId, Person>>,
        usecase: Rc<RefCell<DummyPersonUsecase>>,
        cao: FakePersonCao,
    }
    // フェイクのサービス実装です。ユースケースより先はダミーです。
    impl PersonService<'_, ()> for TargetPersonService {
        type U = DummyPersonUsecase;

        fn run_tx<T, F>(&mut self, f: F) -> Result<T, ServiceError>
        where
            F: FnOnce(&mut Self::U, &mut ()) -> Result<T, UsecaseError>,
        {
            let mut usecase = self.usecase.borrow_mut();
            f(&mut usecase, &mut ()).map_err(ServiceError::TransactionFailed)
        }

        fn register(
            &'_ mut self,
            name: &str,
            birth_date: NaiveDate,
            death_date: Option<NaiveDate>,
            data: &str,
        ) -> Result<(PersonId, Person), ServiceError> {
            let id = *self.next_id.borrow();
            *self.next_id.borrow_mut() += 1;

            let person = Person::new(name, birth_date, death_date, Some(data));

            self.db.borrow_mut().insert(id, person.clone());
            Ok((id, person))
        }

        fn find(&'_ mut self, id: PersonId) -> Result<Option<Person>, ServiceError> {
            Ok(self.db.borrow().get(&id).cloned())
        }

        fn batch_import(&'_ mut self, persons: Vec<Person>) -> Result<Vec<PersonId>, ServiceError> {
            let mut ids = vec![];
            for person in persons {
                let id = *self.next_id.borrow();
                *self.next_id.borrow_mut() += 1;

                self.db.borrow_mut().insert(id, person.clone());
                ids.push(id);
            }
            Ok(ids)
        }

        fn list_all(&'_ mut self) -> Result<Vec<(PersonId, Person)>, ServiceError> {
            Ok(self
                .db
                .borrow()
                .iter()
                .map(|(id, person)| (*id, person.clone()))
                .collect())
        }

        fn unregister(&'_ mut self, id: PersonId) -> Result<(), ServiceError> {
            self.db.borrow_mut().remove(&id);
            Ok(())
        }
    }
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct FakePersonCao {
        cache: Rc<RefCell<HashMap<PersonId, Person>>>,
    }
    impl PersonCao<()> for FakePersonCao {
        fn get_conn(&self) -> Result<(), crate::CaoError> {
            Ok(())
        }
        fn run_tx<T, F>(&self, f: F) -> Result<T, crate::CaoError>
        where
            F: tx_rs::Tx<(), Item = T, Err = crate::CaoError>,
        {
            f.run(&mut ())
        }
        fn exists(&self, id: PersonId) -> impl tx_rs::Tx<(), Item = bool, Err = crate::CaoError> {
            tx_rs::with_tx(move |&mut ()| Ok(self.cache.borrow().contains_key(&id)))
        }
        fn find(
            &self,
            id: PersonId,
        ) -> impl tx_rs::Tx<(), Item = Option<Person>, Err = crate::CaoError> {
            tx_rs::with_tx(move |&mut ()| Ok(self.cache.borrow().get(&id).cloned()))
        }
        fn load(
            &self,
            id: PersonId,
            person: &Person,
        ) -> impl tx_rs::Tx<(), Item = (), Err = crate::CaoError> {
            tx_rs::with_tx(move |&mut ()| {
                self.cache.borrow_mut().insert(id, person.clone());
                Ok(())
            })
        }
        fn unload(&self, id: PersonId) -> impl tx_rs::Tx<(), Item = (), Err = crate::CaoError> {
            tx_rs::with_tx(move |&mut ()| {
                self.cache.borrow_mut().remove(&id);
                Ok(())
            })
        }
    }
    impl PersonCachedService<'_, (), ()> for TargetPersonService {
        type C = FakePersonCao;

        fn get_cao(&self) -> FakePersonCao {
            self.cao.clone()
        }
    }

    #[test]
    fn test_cached_register() {
        let mut service = TargetPersonService {
            next_id: RefCell::new(1),
            db: RefCell::new(HashMap::new()),
            usecase: Rc::new(RefCell::new(DummyPersonUsecase {
                dao: DummyPersonDao,
            })),
            cao: FakePersonCao {
                cache: RefCell::new(HashMap::new()).into(),
            },
        };

        let expected = Person::new("Alice", date(2000, 1, 1), None, Some("Alice is here"));
        let result = service.cached_register("Alice", date(2000, 1, 1), None, "Alice is here");

        assert!(result.is_ok());
        assert_eq!(result, Ok((1, expected)));
    }

    #[test]
    fn test_cached_find() {
        let mut service = TargetPersonService {
            next_id: RefCell::new(1),
            db: RefCell::new(HashMap::new()),
            usecase: Rc::new(RefCell::new(DummyPersonUsecase {
                dao: DummyPersonDao,
            })),
            cao: FakePersonCao {
                cache: RefCell::new(HashMap::new()).into(),
            },
        };

        let result = service.cached_find(1);

        assert!(result.is_ok());
        assert_eq!(result, Ok(None), "not found");

        let mut service = TargetPersonService {
            next_id: RefCell::new(2),
            db: RefCell::new(HashMap::new()),
            usecase: Rc::new(RefCell::new(DummyPersonUsecase {
                dao: DummyPersonDao,
            })),
            cao: FakePersonCao {
                cache: RefCell::new(
                    vec![(
                        1,
                        Person::new("Alice", date(2000, 1, 1), None, Some("Alice is here")),
                    )]
                    .into_iter()
                    .collect(),
                )
                .into(),
            },
        };

        let expected = Person::new("Alice", date(2000, 1, 1), None, Some("Alice is here"));
        let result = service.cached_find(1);

        assert!(result.is_ok());
        assert_eq!(result, Ok(Some(expected)), "hit cache");

        let mut service = TargetPersonService {
            next_id: RefCell::new(2),
            db: RefCell::new(
                vec![(
                    1,
                    Person::new("Alice", date(2000, 1, 1), None, Some("Alice is here")),
                )]
                .into_iter()
                .collect(),
            ),
            usecase: Rc::new(RefCell::new(DummyPersonUsecase {
                dao: DummyPersonDao,
            })),
            cao: FakePersonCao {
                cache: RefCell::new(HashMap::new()).into(),
            },
        };

        let expected = Person::new("Alice", date(2000, 1, 1), None, Some("Alice is here"));
        let result = service.cached_find(1);

        assert!(result.is_ok());
        assert_eq!(result, Ok(Some(expected)), "found db");
    }

    #[test]
    fn test_batch_import() {
        let mut service = TargetPersonService {
            next_id: RefCell::new(1),
            db: RefCell::new(HashMap::new()),
            usecase: Rc::new(RefCell::new(DummyPersonUsecase {
                dao: DummyPersonDao,
            })),
            cao: FakePersonCao {
                cache: RefCell::new(HashMap::new()).into(),
            },
        };

        let result = service.cached_batch_import(vec![
            Person::new("Alice", date(2000, 1, 1), None, Some("Alice is here")),
            Person::new("Bob", date(2000, 1, 2), None, Some("Bob is here")),
        ]);

        assert!(result.is_ok());
        assert_eq!(result, Ok(vec![1, 2]));
    }

    #[test]
    fn test_list_all() {
        let mut service = TargetPersonService {
            next_id: RefCell::new(3),
            db: RefCell::new(
                vec![
                    (
                        1,
                        Person::new("Alice", date(2000, 1, 1), None, Some("Alice is here")),
                    ),
                    (
                        2,
                        Person::new("Bob", date(2000, 1, 2), None, Some("Bob is here")),
                    ),
                ]
                .into_iter()
                .collect(),
            ),
            usecase: Rc::new(RefCell::new(DummyPersonUsecase {
                dao: DummyPersonDao,
            })),
            cao: FakePersonCao {
                cache: RefCell::new(HashMap::new()).into(),
            },
        };

        let result = service.cached_list_all();

        assert!(result.is_ok());
        assert_eq!(result.clone().map(|v| v.len()), Ok(2), "list from db");
    }

    #[test]
    fn test_unregister() {
        let mut service = TargetPersonService {
            next_id: RefCell::new(3),
            db: RefCell::new(
                vec![
                    (
                        1,
                        Person::new("Alice", date(2000, 1, 1), None, Some("Alice is here")),
                    ),
                    (
                        2,
                        Person::new("Bob", date(2000, 1, 2), None, Some("Bob is here")),
                    ),
                ]
                .into_iter()
                .collect(),
            ),
            usecase: Rc::new(RefCell::new(DummyPersonUsecase {
                dao: DummyPersonDao,
            })),
            cao: FakePersonCao {
                cache: RefCell::new(HashMap::new()).into(),
            },
        };

        let result = service.cached_unregister(1);

        assert!(result.is_ok());
        assert_eq!(result, Ok(()));
    }
}

// # スパイテスト(モック利用)
//
// * 目的
//
//   CachedService の各メソッドが、 Cache と Service のメソッドを適切に呼び出していることを保障する
//   つまり、
//    1. 必要なメソッドを必要回数だけ呼び出していること
//    2. 不必要なメソッドを呼び出していないこと
//    3. CachedService に渡った引数が適切に Cache や Service のメソッドに渡されていること
//   を保障する
//
// * 方針
//
//   スパイ Service と スパイ Cache は呼び出されるたびに、それらを全て記録する
//   ただし、 Service の返り値が Cache に使われたりその逆があるため、各スパイは返り値も制御する必要がある
//   よってスタブを兼ねる必要があるため、それぞれをモックとして実装する
//   各メソッドの呼び出された記録をテストの最後で確認する
//
// * 実装
//
//   1. ダミーの DAO 構造体、ユースケース構造体を用意する
//      この構造体は実質使われないが、 Service に必要なので用意する
//   2. メソッド呼び出しを記録しつつ、設定された返り値を返すモック Service を実装する
//   3. メソッド呼び出しを記録しつつ、設定された返り値を返すモック Cache を実装する
//   4. CachedService をここまでに用意したモックとダミーで構築する
//   5. Service のメソッドを呼び出す
//   6. Cache と Service の記録を検証する
//
// * 注意
//
//   1. このテストは CachedService の実装を保障するものであって、Service や Cache の実装を保障するものではない
//   2. このテストは CachedService のメソッドが不適切な Cache メソッドや Service メソッド呼び出しをしていないことを保障するものであって Cache や Service の不適切な処理をしていないことを保障するものではない
//   3. このテストでは Cache と Service のメソッド呼び出し順序については検証しない (将来的に検証することを拒否しない)
#[cfg(test)]
mod spy_tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use crate::{
        dao::{DaoError, PersonDao},
        date, HavePersonDao, PersonUsecase, UsecaseError,
    };

    use super::*;

    struct DummyPersonDao;
    impl PersonDao<()> for DummyPersonDao {
        fn insert(&self, _person: Person) -> impl tx_rs::Tx<(), Item = PersonId, Err = DaoError> {
            tx_rs::with_tx(move |&mut ()| Ok(1))
        }
        fn fetch(
            &self,
            _id: PersonId,
        ) -> impl tx_rs::Tx<(), Item = Option<Person>, Err = DaoError> {
            tx_rs::with_tx(move |&mut ()| Ok(None))
        }
        fn select(&self) -> impl tx_rs::Tx<(), Item = Vec<(PersonId, Person)>, Err = DaoError> {
            tx_rs::with_tx(move |&mut ()| Ok(vec![]))
        }
        fn delete(&self, _id: PersonId) -> impl tx_rs::Tx<(), Item = (), Err = DaoError> {
            tx_rs::with_tx(move |&mut ()| Ok(()))
        }
    }

    struct DummyPersonUsecase {
        dao: DummyPersonDao,
    }
    impl HavePersonDao<()> for DummyPersonUsecase {
        fn get_dao<'b>(&'b self) -> Box<&impl PersonDao<()>> {
            Box::new(&self.dao)
        }
    }
    impl PersonUsecase<()> for DummyPersonUsecase {
        fn entry<'a>(
            &'a mut self,
            _person: Person,
        ) -> impl tx_rs::Tx<(), Item = PersonId, Err = UsecaseError>
        where
            (): 'a,
        {
            tx_rs::with_tx(move |&mut ()| Ok(1))
        }
        fn find<'a>(
            &'a mut self,
            _id: PersonId,
        ) -> impl tx_rs::Tx<(), Item = Option<Person>, Err = UsecaseError>
        where
            (): 'a,
        {
            tx_rs::with_tx(move |&mut ()| Ok(None))
        }
        fn entry_and_verify<'a>(
            &'a mut self,
            person: Person,
        ) -> impl tx_rs::Tx<(), Item = (PersonId, Person), Err = UsecaseError>
        where
            (): 'a,
        {
            tx_rs::with_tx(move |&mut ()| Ok((1, person)))
        }
        fn collect<'a>(
            &'a mut self,
        ) -> impl tx_rs::Tx<(), Item = Vec<(PersonId, Person)>, Err = UsecaseError>
        where
            (): 'a,
        {
            tx_rs::with_tx(move |&mut ()| Ok(vec![]))
        }
        fn remove<'a>(
            &'a mut self,
            _id: PersonId,
        ) -> impl tx_rs::Tx<(), Item = (), Err = UsecaseError>
        where
            (): 'a,
        {
            tx_rs::with_tx(move |&mut ()| Ok(()))
        }
    }

    /// テスト用のモックサービスです。
    struct TargetPersonService {
        register: RefCell<Vec<(String, NaiveDate, Option<NaiveDate>, Option<String>)>>,
        register_result: Result<(PersonId, Person), ServiceError>,
        find: RefCell<Vec<PersonId>>,
        find_result: Result<Option<Person>, ServiceError>,
        batch_import: RefCell<Vec<Vec<Person>>>,
        batch_import_result: Result<Vec<PersonId>, ServiceError>,
        list_all: RefCell<i32>,
        list_all_result: Result<Vec<(PersonId, Person)>, ServiceError>,
        unregister: RefCell<Vec<PersonId>>,
        unregister_result: Result<(), ServiceError>,

        usecase: RefCell<DummyPersonUsecase>,
        cao: MockPersonCao,
    }
    // モックサービス実装です。ユースケースより先はダミーです。
    impl PersonService<'_, ()> for TargetPersonService {
        type U = DummyPersonUsecase;

        fn run_tx<T, F>(&mut self, f: F) -> Result<T, ServiceError>
        where
            F: FnOnce(&mut Self::U, &mut ()) -> Result<T, UsecaseError>,
        {
            let mut usecase = self.usecase.borrow_mut();
            f(&mut usecase, &mut ()).map_err(ServiceError::TransactionFailed)
        }

        fn register(
            &'_ mut self,
            name: &str,
            birth_date: NaiveDate,
            death_date: Option<NaiveDate>,
            data: &str,
        ) -> Result<(PersonId, Person), ServiceError> {
            self.register.borrow_mut().push((
                name.to_string(),
                birth_date,
                death_date,
                Some(data.to_string()),
            ));
            self.register_result.clone()
        }

        fn find(&'_ mut self, id: PersonId) -> Result<Option<Person>, ServiceError> {
            self.find.borrow_mut().push(id);
            self.find_result.clone()
        }

        fn batch_import(&'_ mut self, persons: Vec<Person>) -> Result<Vec<PersonId>, ServiceError> {
            self.batch_import.borrow_mut().push(persons);
            self.batch_import_result.clone()
        }

        fn list_all(&'_ mut self) -> Result<Vec<(PersonId, Person)>, ServiceError> {
            *self.list_all.borrow_mut() += 1;
            self.list_all_result.clone()
        }

        fn unregister(&'_ mut self, id: PersonId) -> Result<(), ServiceError> {
            self.unregister.borrow_mut().push(id);
            self.unregister_result.clone()
        }
    }
    // モックキャッシュ実装です
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct MockPersonCao {
        exists: Rc<RefCell<Vec<PersonId>>>,
        exists_result: Result<bool, crate::CaoError>,
        find: Rc<RefCell<Vec<PersonId>>>,
        find_result: Result<Option<Person>, crate::CaoError>,
        load: Rc<RefCell<Vec<(PersonId, Person)>>>,
        load_result: Result<(), crate::CaoError>,
        unload: Rc<RefCell<Vec<PersonId>>>,
        unload_result: Result<(), crate::CaoError>,
    }
    impl PersonCao<()> for MockPersonCao {
        fn get_conn(&self) -> Result<(), crate::CaoError> {
            Ok(())
        }
        fn run_tx<T, F>(&self, f: F) -> Result<T, crate::CaoError>
        where
            F: tx_rs::Tx<(), Item = T, Err = crate::CaoError>,
        {
            f.run(&mut ())
        }
        fn exists(&self, id: PersonId) -> impl tx_rs::Tx<(), Item = bool, Err = crate::CaoError> {
            tx_rs::with_tx(move |&mut ()| {
                self.exists.borrow_mut().push(id);
                self.exists_result.clone()
            })
        }
        fn find(
            &self,
            id: PersonId,
        ) -> impl tx_rs::Tx<(), Item = Option<Person>, Err = crate::CaoError> {
            tx_rs::with_tx(move |&mut ()| {
                self.find.borrow_mut().push(id);
                self.find_result.clone()
            })
        }
        fn load(
            &self,
            id: PersonId,
            person: &Person,
        ) -> impl tx_rs::Tx<(), Item = (), Err = crate::CaoError> {
            tx_rs::with_tx(move |&mut ()| {
                self.load.borrow_mut().push((id, person.clone()));
                self.load_result.clone()
            })
        }
        fn unload(&self, id: PersonId) -> impl tx_rs::Tx<(), Item = (), Err = crate::CaoError> {
            tx_rs::with_tx(move |&mut ()| {
                self.unload.borrow_mut().push(id);
                self.unload_result.clone()
            })
        }
    }
    impl PersonCachedService<'_, (), ()> for TargetPersonService {
        type C = MockPersonCao;

        fn get_cao(&self) -> MockPersonCao {
            self.cao.clone()
        }
    }

    #[test]
    fn test_cached_register() {
        let mut service = TargetPersonService {
            register: RefCell::new(vec![]),
            register_result: Ok((
                1,
                Person::new("Alice", date(2000, 1, 1), None, Some("Alice is here")),
            )),
            find: RefCell::new(vec![]),
            find_result: Ok(None), // 使われない
            batch_import: RefCell::new(vec![]),
            batch_import_result: Ok(vec![]), // 使われない
            list_all: RefCell::new(0),
            list_all_result: Ok(vec![]), // 使われない
            unregister: RefCell::new(vec![]),
            unregister_result: Ok(()), // 使われない
            usecase: RefCell::new(DummyPersonUsecase {
                dao: DummyPersonDao,
            }),
            cao: MockPersonCao {
                exists: Rc::new(RefCell::new(vec![])),
                exists_result: Ok(false), // 使われない
                find: Rc::new(RefCell::new(vec![])),
                find_result: Ok(None), // 使われない
                load: Rc::new(RefCell::new(vec![])),
                load_result: Ok(()), // 使われない
                unload: Rc::new(RefCell::new(vec![])),
                unload_result: Ok(()), // 使われない
            },
        };

        let _ = service.cached_register("Alice", date(2000, 1, 1), None, "Alice is here");
        assert_eq!(
            *service.register.borrow(),
            vec![(
                "Alice".to_string(),
                date(2000, 1, 1),
                None,
                Some("Alice is here".to_string())
            )]
        );
        assert_eq!(*service.find.borrow(), vec![] as Vec<PersonId>);
        assert_eq!(*service.batch_import.borrow(), vec![] as Vec<Vec<Person>>);
        assert_eq!(*service.list_all.borrow(), 0);
        assert_eq!(*service.unregister.borrow(), vec![] as Vec<PersonId>);

        assert_eq!(*service.cao.exists.borrow(), vec![] as Vec<PersonId>);
        assert_eq!(*service.cao.find.borrow(), vec![] as Vec<PersonId>);
        assert_eq!(
            *service.cao.load.borrow(),
            vec![(
                1,
                Person::new("Alice", date(2000, 1, 1), None, Some("Alice is here"))
            )]
        );
        assert_eq!(*service.cao.unload.borrow(), vec![] as Vec<PersonId>);
    }

    #[test]
    fn test_cached_find() {
        let mut service = TargetPersonService {
            register: RefCell::new(vec![]),
            register_result: Ok((1, Person::new("", date(2000, 1, 1), None, Some("")))), // 使われない
            find: RefCell::new(vec![]),
            find_result: Ok(None), // 使われない
            batch_import: RefCell::new(vec![]),
            batch_import_result: Ok(vec![]), // 使われない
            list_all: RefCell::new(0),
            list_all_result: Ok(vec![]), // 使われない
            unregister: RefCell::new(vec![]),
            unregister_result: Ok(()), // 使われない
            usecase: RefCell::new(DummyPersonUsecase {
                dao: DummyPersonDao,
            }),
            cao: MockPersonCao {
                exists: Rc::new(RefCell::new(vec![])),
                exists_result: Ok(false), // 使われない
                find: Rc::new(RefCell::new(vec![])),
                find_result: Ok(Some(Person::new(
                    "Alice",
                    date(2000, 1, 1),
                    None,
                    Some("Alice is here"),
                ))),
                load: Rc::new(RefCell::new(vec![])),
                load_result: Ok(()), // 使われない
                unload: Rc::new(RefCell::new(vec![])),
                unload_result: Ok(()), // 使われない
            },
        };

        let _ = service.cached_find(1);
        assert_eq!(*service.register.borrow(), vec![]);
        assert_eq!(*service.find.borrow(), vec![] as Vec<PersonId>);
        assert_eq!(*service.batch_import.borrow(), vec![] as Vec<Vec<Person>>);
        assert_eq!(*service.list_all.borrow(), 0);
        assert_eq!(*service.unregister.borrow(), vec![] as Vec<PersonId>);

        assert_eq!(*service.cao.exists.borrow(), vec![] as Vec<PersonId>);
        assert_eq!(*service.cao.find.borrow(), vec![1]);
        assert_eq!(
            *service.cao.load.borrow(),
            vec![] as Vec<(PersonId, Person)>
        );
        assert_eq!(*service.cao.unload.borrow(), vec![] as Vec<PersonId>);

        let mut service = TargetPersonService {
            register: RefCell::new(vec![]),
            register_result: Ok((1, Person::new("", date(2000, 1, 1), None, Some("")))), // 使われない
            find: RefCell::new(vec![]),
            find_result: Ok(Some(Person::new(
                "Alice",
                date(2000, 1, 1),
                None,
                Some("Alice is here"),
            ))),
            batch_import: RefCell::new(vec![]),
            batch_import_result: Ok(vec![]), // 使われない
            list_all: RefCell::new(0),
            list_all_result: Ok(vec![]), // 使われない
            unregister: RefCell::new(vec![]),
            unregister_result: Ok(()), // 使われない
            usecase: RefCell::new(DummyPersonUsecase {
                dao: DummyPersonDao,
            }),
            cao: MockPersonCao {
                exists: Rc::new(RefCell::new(vec![])),
                exists_result: Ok(false), // 使われない
                find: Rc::new(RefCell::new(vec![])),
                find_result: Ok(None),
                load: Rc::new(RefCell::new(vec![])),
                load_result: Ok(()), // 使われない
                unload: Rc::new(RefCell::new(vec![])),
                unload_result: Ok(()), // 使われない
            },
        };

        let _ = service.cached_find(1);
        assert_eq!(*service.register.borrow(), vec![]);
        assert_eq!(*service.find.borrow(), vec![1]);
        assert_eq!(*service.batch_import.borrow(), vec![] as Vec<Vec<Person>>);
        assert_eq!(*service.list_all.borrow(), 0);
        assert_eq!(*service.unregister.borrow(), vec![] as Vec<PersonId>);

        assert_eq!(*service.cao.exists.borrow(), vec![] as Vec<PersonId>);
        assert_eq!(*service.cao.find.borrow(), vec![1]);
        assert_eq!(
            *service.cao.load.borrow(),
            vec![(
                1,
                Person::new("Alice", date(2000, 1, 1), None, Some("Alice is here"))
            )]
        );
        assert_eq!(*service.cao.unload.borrow(), vec![] as Vec<PersonId>);
    }

    #[test]
    fn test_batch_import() {
        let mut service = TargetPersonService {
            register: RefCell::new(vec![]),
            register_result: Ok((1, Person::new("", date(2000, 1, 1), None, Some("")))), // 使われない
            find: RefCell::new(vec![]),
            find_result: Ok(None), // 使われない
            batch_import: RefCell::new(vec![]),
            batch_import_result: Ok(vec![3, 4, 5]),
            list_all: RefCell::new(0),
            list_all_result: Ok(vec![]), // 使われない
            unregister: RefCell::new(vec![]),
            unregister_result: Ok(()), // 使われない
            usecase: RefCell::new(DummyPersonUsecase {
                dao: DummyPersonDao,
            }),
            cao: MockPersonCao {
                exists: Rc::new(RefCell::new(vec![])),
                exists_result: Ok(false), // 使われない
                find: Rc::new(RefCell::new(vec![])),
                find_result: Ok(None), // 使われない
                load: Rc::new(RefCell::new(vec![])),
                load_result: Ok(()), // 使われない
                unload: Rc::new(RefCell::new(vec![])),
                unload_result: Ok(()), // 使われない
            },
        };

        let _ = service.cached_batch_import(vec![
            Person::new("Alice", date(2000, 1, 1), None, Some("Alice is sender")),
            Person::new("Bob", date(2001, 2, 2), None, Some("Bob is receiver")),
            Person::new("Eve", date(2002, 3, 3), None, Some("Eve is interceptor")),
        ]);
        assert_eq!(*service.register.borrow(), vec![]);
        assert_eq!(*service.find.borrow(), vec![] as Vec<PersonId>);
        assert_eq!(
            *service.batch_import.borrow(),
            vec![vec![
                Person::new("Alice", date(2000, 1, 1), None, Some("Alice is sender")),
                Person::new("Bob", date(2001, 2, 2), None, Some("Bob is receiver")),
                Person::new("Eve", date(2002, 3, 3), None, Some("Eve is interceptor")),
            ]]
        );
        assert_eq!(*service.list_all.borrow(), 0);
        assert_eq!(*service.unregister.borrow(), vec![] as Vec<PersonId>);

        assert_eq!(*service.cao.exists.borrow(), vec![] as Vec<PersonId>);
        assert_eq!(*service.cao.find.borrow(), vec![] as Vec<PersonId>);
        assert_eq!(
            *service.cao.load.borrow(),
            vec![
                (
                    3,
                    Person::new("Alice", date(2000, 1, 1), None, Some("Alice is sender"))
                ),
                (
                    4,
                    Person::new("Bob", date(2001, 2, 2), None, Some("Bob is receiver"))
                ),
                (
                    5,
                    Person::new("Eve", date(2002, 3, 3), None, Some("Eve is interceptor"))
                ),
            ]
        );
        assert_eq!(*service.cao.unload.borrow(), vec![] as Vec<PersonId>);
    }
}
