use log::{trace, warn};
use thiserror::Error;

use crate::dao::{DaoError, HavePersonDao, PersonDao};
use crate::domain::{Person, PersonId};
use tx_rs::Tx;

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum UsecaseError {
    #[error("entry person failed: {0}")]
    EntryPersonFailed(DaoError),
    #[error("find person failed: {0}")]
    FindPersonFailed(DaoError),
    #[error("entry and verify failed: {0}")]
    EntryAndVerifyPersonFailed(DaoError),
    #[error("collect person failed: {0}")]
    CollectPersonFailed(DaoError),
    #[error("remove person failed: {0}")]
    RemovePersonFailed(DaoError),
}
pub trait PersonUsecase<Ctx>: HavePersonDao<Ctx> {
    fn entry<'a>(
        &'a mut self,
        person: Person,
    ) -> impl tx_rs::Tx<Ctx, Item = PersonId, Err = UsecaseError>
    where
        Ctx: 'a,
    {
        let dao = self.get_dao();
        trace!("insert person: {:?}", person);
        dao.insert(person)
            .map_err(|e| UsecaseError::EntryPersonFailed(e))
    }
    fn find<'a>(
        &'a mut self,
        id: PersonId,
    ) -> impl tx_rs::Tx<Ctx, Item = Option<Person>, Err = UsecaseError>
    where
        Ctx: 'a,
    {
        let dao = self.get_dao();
        trace!("find person_id: {:?}", id);
        dao.fetch(id).map_err(|e| UsecaseError::FindPersonFailed(e))
    }
    fn entry_and_verify<'a>(
        &'a mut self,
        person: Person,
    ) -> impl tx_rs::Tx<Ctx, Item = (PersonId, Person), Err = UsecaseError>
    where
        Ctx: 'a,
    {
        let dao = self.get_dao();
        trace!("entry and verify person: {:?}", person);
        dao.insert(person)
            .and_then(move |id| {
                dao.fetch(id).try_map(move |person| {
                    if let Some(p) = person {
                        return Ok((id, p));
                    }

                    warn!("can't find the person just entried: {}", id);
                    Err(DaoError::SelectError(
                        format!("not found: {id}").to_string(),
                    ))
                })
            })
            .map_err(|e| UsecaseError::EntryAndVerifyPersonFailed(e))
    }
    fn collect<'a>(
        &'a mut self,
    ) -> impl tx_rs::Tx<Ctx, Item = Vec<(PersonId, Person)>, Err = UsecaseError>
    where
        Ctx: 'a,
    {
        let dao = self.get_dao();
        trace!("collect all persons");
        dao.select()
            .map_err(|e| UsecaseError::CollectPersonFailed(e))
    }
    fn remove<'a>(&'a mut self, id: PersonId) -> impl tx_rs::Tx<Ctx, Item = (), Err = UsecaseError>
    where
        Ctx: 'a,
    {
        let dao = self.get_dao();
        trace!("remove person_id: {:?}", id);
        dao.delete(id)
            .map_err(|e| UsecaseError::RemovePersonFailed(e))
    }
}

// # フェイクテスト
//
// * 目的
//
//   Usecase の正常系のテストを行う
//   Usecase の各メソッドが DAO から通常期待される結果を受け取ったときに適切にふるまうことを保障する
//
// * 方針
//
//   DAO のフェイクに対して Usecase を実行し、その結果を確認する
//   フェイクはテスト時の比較チェックのしやすさを考慮して HashMap ではなく Vec で登録データを保持する
//   データ数は多くないので、Vec でリニアサーチしても十分な速度が出ると考える
//
// * 実装
//
//   1. DAO のメソッド呼び出しに対して、期待される結果を返す DAO 構造体を用意する
//      この DAO 構造体はフェイクなので、間接的な入力と間接的な出力が整合するようにする
//   2. Usecase にそのフェイクをプラグインする
//   3. Usecase のメソッドを呼び出す
//   4. Usecase からの戻り値を検証する
//
// * 注意
//
//   1. このテストは Usecase の実装を保障するものであって、DAO の実装を保障するものではない
//
#[cfg(test)]
mod fake_tests {
    use std::cell::RefCell;

    use super::*;
    use crate::domain::date;

    struct FakePersonDao {
        last_id: RefCell<PersonId>,
        data: RefCell<Vec<(PersonId, Person)>>,
    }
    // Ctx 不要なので () にしている
    impl PersonDao<()> for FakePersonDao {
        fn insert(&self, person: Person) -> impl tx_rs::Tx<(), Item = PersonId, Err = DaoError> {
            *self.last_id.borrow_mut() += 1;
            let id = *self.last_id.borrow();
            self.data.borrow_mut().push((id, person));

            tx_rs::with_tx(move |()| Ok(id))
        }
        fn fetch(&self, id: PersonId) -> impl tx_rs::Tx<(), Item = Option<Person>, Err = DaoError> {
            let data = self.data.borrow();
            let result = data.iter().find(|(i, _)| *i == id).map(|(_, p)| p.clone());

            tx_rs::with_tx(move |()| Ok(result))
        }
        fn select(&self) -> impl tx_rs::Tx<(), Item = Vec<(PersonId, Person)>, Err = DaoError> {
            let result = self.data.borrow().clone();

            tx_rs::with_tx(move |()| Ok(result))
        }
        fn delete(&self, id: PersonId) -> impl tx_rs::Tx<(), Item = (), Err = DaoError> {
            let result = self.data.borrow_mut().retain(|(i, _)| *i != id);

            tx_rs::with_tx(move |()| Ok(result))
        }
    }

    struct TargetPersonUsecase {
        dao: FakePersonDao,
    }
    impl HavePersonDao<()> for TargetPersonUsecase {
        fn get_dao(&self) -> Box<&impl PersonDao<()>> {
            Box::new(&self.dao)
        }
    }
    impl PersonUsecase<()> for TargetPersonUsecase {}

    #[test]
    fn test_entry() {
        let dao = FakePersonDao {
            last_id: RefCell::new(0),
            data: RefCell::new(vec![]),
        };
        let mut usecase = TargetPersonUsecase { dao };

        let person = Person::new("Alice", date(2012, 11, 2), None, Some("Alice wonderland"));
        let expected = person.clone();
        let expected_id = 1;

        let result = usecase.entry(person).run(&mut ());
        assert_eq!(result, Ok(expected_id));
        assert_eq!(usecase.dao.data.borrow().len(), expected_id as usize);
        assert_eq!(*usecase.dao.data.borrow(), vec![(expected_id, expected)]);
    }
    #[test]
    fn test_find() {
        let dao = FakePersonDao {
            last_id: RefCell::new(0), // 使わない
            data: RefCell::new(vec![
                (
                    13,
                    Person::new("Alice", date(2012, 11, 2), None, Some("Alice is sender")),
                ),
                (
                    24,
                    Person::new("Bob", date(1995, 11, 6), None, Some("Bob is receiver")),
                ),
                (
                    99,
                    Person::new("Eve", date(1996, 12, 15), None, Some("Eve is interceptor")),
                ),
            ]),
        };
        let mut usecase = TargetPersonUsecase { dao };

        let result = usecase.find(13).run(&mut ());
        assert_eq!(
            result,
            Ok(Some(Person::new(
                "Alice",
                date(2012, 11, 2),
                None,
                Some("Alice is sender")
            )))
        );
    }
    #[test]
    fn test_entry_and_verify() {
        let dao = FakePersonDao {
            last_id: RefCell::new(13),
            data: RefCell::new(vec![]),
        };
        let mut usecase = TargetPersonUsecase { dao };

        let person = Person::new("Alice", date(2012, 11, 2), None, Some("Alice wonderland"));
        let expected = person.clone();
        let expected_id = 14;

        let result = usecase.entry_and_verify(person).run(&mut ());
        assert_eq!(result, Ok((expected_id, expected)));
    }
    #[test]
    fn test_collect() {
        let data = vec![
            (
                13,
                Person::new("Alice", date(2012, 11, 2), None, Some("Alice is sender")),
            ),
            (
                24,
                Person::new("Bob", date(1995, 11, 6), None, Some("Bob is receiver")),
            ),
            (
                99,
                Person::new("Eve", date(1996, 12, 15), None, Some("Eve is interceptor")),
            ),
        ];
        let expected = data.clone();

        let dao = FakePersonDao {
            last_id: RefCell::new(0), // 使わない
            data: RefCell::new(data),
        };
        let mut usecase = TargetPersonUsecase { dao };

        let result = usecase.collect().run(&mut ());
        assert_eq!(
            result.map(|mut v: Vec<(PersonId, Person)>| {
                v.sort_by_key(|(id, _)| *id);
                v
            }),
            Ok(expected)
        );
    }
    #[test]
    fn test_remove() {
        let data = vec![
            (
                13,
                Person::new("Alice", date(2012, 11, 2), None, Some("Alice is sender")),
            ),
            (
                24,
                Person::new("Bob", date(1995, 11, 6), None, Some("Bob is receiver")),
            ),
            (
                99,
                Person::new("Eve", date(1996, 12, 15), None, Some("Eve is interceptor")),
            ),
        ];
        let expected = vec![
            (
                13,
                Person::new("Alice", date(2012, 11, 2), None, Some("Alice is sender")),
            ),
            (
                99,
                Person::new("Eve", date(1996, 12, 15), None, Some("Eve is interceptor")),
            ),
        ];

        let dao = FakePersonDao {
            last_id: RefCell::new(0), // 使わない
            data: RefCell::new(data),
        };
        let mut usecase = TargetPersonUsecase { dao };

        let result = usecase.remove(24).run(&mut ());
        assert_eq!(result, Ok(()));
        assert_eq!(*usecase.dao.data.borrow(), expected);
    }
}

// # スパイテスト
//
// * 目的
//
//   Usecase の各メソッドが DAO のメソッドを適切に呼び出していることを保障する
//   つまり、
//    1. 必要なメソッドを必要回数だけ呼び出していること
//    2. 不必要なメソッドを呼び出していないこと
//    3. Usecase に渡った引数が適切に DAO のメソッドに渡されていること
//   を保障する
//
//
// * 方針
//
//   DAO のメソッドを呼び出すたびに、その呼び出しを記録する
//   その記録をテストの最後で確認する
//
// * 実装
//
//   1. DAO のメソッド呼び出しを記録する種類の DAO 構造体を用意する
//      この構造体はスパイなので、Usecase の間接的な出力のみ記録する
//   2. その構造体を Usecase にプラグインする
//   3. Usecase のメソッドを呼び出す
//   4. その後で DAO 構造体の記録を検証する
//
// * 注意
//
//   1. このテストは Usecase の実装を保障するものであって、DAO の実装を保障するものではない
//   2. このテストは Usecase のメソッドが適切に DAO のメソッドを呼び出していることを保障するものであって、
//      DAO のメソッドが適切にデータベースを操作していることを保障するものではない
//   3. このテストは Usecase のメソッドが不適切な DAO のメソッド呼び出しをしていないことを保障するものであって、
//      DAO のメソッドが不適切なデータベースの操作をしていないことを保障するものではない
//   4. このテストでは DAO のメソッドの呼び出し順序については検証しない (将来的に検証することは拒否しない)
//
#[cfg(test)]
mod spy_tests {
    use std::cell::RefCell;

    use super::*;
    use crate::domain::date;

    struct SpyPersonDao {
        insert: RefCell<Vec<Person>>,
        inserted_id: PersonId,
        fetch: RefCell<Vec<PersonId>>,
        select: RefCell<i32>,
        delete: RefCell<Vec<PersonId>>,
    }
    // Ctx 不要なので () にしている
    impl PersonDao<()> for SpyPersonDao {
        fn insert(&self, person: Person) -> impl tx_rs::Tx<(), Item = PersonId, Err = DaoError> {
            self.insert.borrow_mut().push(person);

            // 返り値には意味なし
            tx_rs::with_tx(|()| Ok(42 as PersonId))
        }
        fn fetch(&self, id: PersonId) -> impl tx_rs::Tx<(), Item = Option<Person>, Err = DaoError> {
            self.fetch.borrow_mut().push(id);

            // 返り値には意味なし
            tx_rs::with_tx(|()| Ok(None))
        }
        fn select(&self) -> impl tx_rs::Tx<(), Item = Vec<(PersonId, Person)>, Err = DaoError> {
            *self.select.borrow_mut() += 1;

            // 返り値には意味なし
            tx_rs::with_tx(|()| Ok(vec![]))
        }
        fn delete(&self, id: PersonId) -> impl tx_rs::Tx<(), Item = (), Err = DaoError> {
            self.delete.borrow_mut().push(id);

            // 返り値には意味なし
            tx_rs::with_tx(|()| Ok(()))
        }
    }

    struct TargetPersonUsecase {
        dao: SpyPersonDao,
    }
    impl HavePersonDao<()> for TargetPersonUsecase {
        fn get_dao(&self) -> Box<&impl PersonDao<()>> {
            Box::new(&self.dao)
        }
    }
    impl PersonUsecase<()> for TargetPersonUsecase {}

    #[test]
    fn test_entry() {
        let dao = SpyPersonDao {
            insert: RefCell::new(vec![]),
            inserted_id: 0, // 使わない
            fetch: RefCell::new(vec![]),
            select: RefCell::new(0),
            delete: RefCell::new(vec![]),
        };
        let mut usecase = TargetPersonUsecase { dao };

        let person = Person::new("Alice", date(2012, 11, 2), None, None);
        let expected = person.clone();

        let _ = usecase.entry(person).run(&mut ()).unwrap();

        // DAO のメソッドの呼び出し記録の検証
        assert_eq!(usecase.dao.insert.borrow().len(), 1);
        assert_eq!(usecase.dao.fetch.borrow().len(), 0);
        assert_eq!(*usecase.dao.select.borrow(), 0);
        assert_eq!(usecase.dao.delete.borrow().len(), 0);

        // Usecase の引数が DAO にそのまま渡されていることを検証
        assert_eq!(usecase.dao.insert.borrow()[0], expected);
    }

    #[test]
    fn test_find() {
        let dao = SpyPersonDao {
            insert: RefCell::new(vec![]),
            inserted_id: 0, // 使わない
            fetch: RefCell::new(vec![]),
            select: RefCell::new(0),
            delete: RefCell::new(vec![]),
        };
        let mut usecase = TargetPersonUsecase { dao };

        let id: PersonId = 42;
        let expected = id;
        let _ = usecase.find(id).run(&mut ());

        // DAO のメソッドの呼び出し記録の検証
        assert_eq!(usecase.dao.insert.borrow().len(), 0);
        assert_eq!(usecase.dao.fetch.borrow().len(), 1);
        assert_eq!(*usecase.dao.select.borrow(), 0);
        assert_eq!(usecase.dao.delete.borrow().len(), 0);

        // Usecase の引数が DAO にそのまま渡されていることを確認
        assert_eq!(usecase.dao.fetch.borrow()[0], expected);
    }

    #[test]
    fn test_entry_and_verify() {
        let dao = SpyPersonDao {
            insert: RefCell::new(vec![]),
            inserted_id: 42,
            fetch: RefCell::new(vec![]),
            select: RefCell::new(0),
            delete: RefCell::new(vec![]),
        };
        let mut usecase = TargetPersonUsecase { dao };

        let person = Person::new("Alice", date(2012, 11, 2), None, None);
        let expected = person.clone();

        let _ = usecase.entry_and_verify(person).run(&mut ());

        // DAO のメソッドの呼び出し記録の検証
        assert_eq!(usecase.dao.insert.borrow().len(), 1);
        assert_eq!(usecase.dao.fetch.borrow().len(), 1);
        assert_eq!(*usecase.dao.select.borrow(), 0);
        assert_eq!(usecase.dao.delete.borrow().len(), 0);

        // Usecase の引数が DAO にそのまま渡されていることを検証
        assert_eq!(usecase.dao.insert.borrow()[0], expected);
        // insert で返された ID が fetch に渡されていることを検証
        assert_eq!(usecase.dao.fetch.borrow()[0], usecase.dao.inserted_id);
    }

    #[test]
    fn test_collect() {
        let dao = SpyPersonDao {
            insert: RefCell::new(vec![]),
            inserted_id: 0, // 使わない
            fetch: RefCell::new(vec![]),
            select: RefCell::new(0),
            delete: RefCell::new(vec![]),
        };
        let mut usecase = TargetPersonUsecase { dao };

        let _ = usecase.collect().run(&mut ());

        // DAO のメソッドの呼び出し記録の検証
        assert_eq!(usecase.dao.insert.borrow().len(), 0);
        assert_eq!(usecase.dao.fetch.borrow().len(), 0);
        assert_eq!(*usecase.dao.select.borrow(), 1);
        assert_eq!(usecase.dao.delete.borrow().len(), 0);
    }
    #[test]
    fn test_remove() {
        let dao = SpyPersonDao {
            insert: RefCell::new(vec![]),
            inserted_id: 0, // 使わない
            fetch: RefCell::new(vec![]),
            select: RefCell::new(0),
            delete: RefCell::new(vec![]),
        };
        let mut usecase = TargetPersonUsecase { dao };

        let id: PersonId = 42;
        let expected = id;
        let _ = usecase.remove(id).run(&mut ());

        // DAO のメソッドの呼び出し記録の検証
        assert_eq!(usecase.dao.insert.borrow().len(), 0);
        assert_eq!(usecase.dao.fetch.borrow().len(), 0);
        assert_eq!(*usecase.dao.select.borrow(), 0);
        assert_eq!(usecase.dao.delete.borrow().len(), 1);

        // Usecase の引数が DAO にそのまま渡されていることを確認
        assert_eq!(usecase.dao.delete.borrow()[0], expected);
    }
}

// # エラー系スタブテスト
//
// * 目的
//
//   DAO がエラーを返した場合の Usecase の挙動を保障する
//
// * 方針
//
//   DAO の各メソッドで任意を結果を返せるようにして Usecase のメソッドを呼び出して Usecase の結果を確認する
//
// * 実装
//
//   1. DAO のメソッドが任意の結果を返せる種類の DAO 構造体を用意する
//      この DAO 構造体はスタブであり、Usecase への間接的な入力のみ制御する
//   2. その構造体を Usecase にプラグインする
//   3. Usecase のメソッドを呼び出す
//   4. Usecase のメソッドからの戻り値を確認する
//
// * 注意
//
#[cfg(test)]
mod error_stub_tests {
    use super::*;
    use crate::domain::date;

    struct StubPersonDao {
        insert_result: Result<PersonId, DaoError>,
        fetch_result: Result<Option<Person>, DaoError>,
        select_result: Result<Vec<(PersonId, Person)>, DaoError>,
        delete_result: Result<(), DaoError>,
    }
    // Ctx 不要なので () にしている
    impl PersonDao<()> for StubPersonDao {
        fn insert(&self, _person: Person) -> impl tx_rs::Tx<(), Item = PersonId, Err = DaoError> {
            tx_rs::with_tx(|()| self.insert_result.clone())
        }
        fn fetch(
            &self,
            _id: PersonId,
        ) -> impl tx_rs::Tx<(), Item = Option<Person>, Err = DaoError> {
            tx_rs::with_tx(|()| self.fetch_result.clone())
        }
        fn select(&self) -> impl tx_rs::Tx<(), Item = Vec<(PersonId, Person)>, Err = DaoError> {
            tx_rs::with_tx(|()| self.select_result.clone())
        }
        fn delete(&self, _id: PersonId) -> impl tx_rs::Tx<(), Item = (), Err = DaoError> {
            tx_rs::with_tx(move |()| self.delete_result.clone())
        }
    }

    struct TargetPersonUsecase {
        dao: StubPersonDao,
    }
    impl HavePersonDao<()> for TargetPersonUsecase {
        fn get_dao(&self) -> Box<&impl PersonDao<()>> {
            Box::new(&self.dao)
        }
    }
    impl PersonUsecase<()> for TargetPersonUsecase {}

    #[test]
    fn test_entry() {
        let dao = StubPersonDao {
            insert_result: Err(DaoError::InsertError("valid dao".to_string())),
            fetch_result: Ok(None),    // 使わない
            select_result: Ok(vec![]), // 使わない
            delete_result: Ok(()),     // 使わない
        };
        let expected = UsecaseError::EntryPersonFailed(dao.insert_result.clone().unwrap_err());

        let mut usecase = TargetPersonUsecase { dao };

        let person = Person::new("Alice", date(2012, 11, 2), None, None);
        let result = usecase.entry(person).run(&mut ());

        assert!(result.is_err());
        assert_eq!(result.err().unwrap(), expected);
    }

    #[test]
    fn test_find() {
        let dao = StubPersonDao {
            insert_result: Ok(42), // 使わない
            fetch_result: Err(DaoError::SelectError("valid dao".to_string())),
            select_result: Ok(vec![]), // 使わない
            delete_result: Ok(()),     // 使わない
        };
        let expected = UsecaseError::FindPersonFailed(dao.fetch_result.clone().unwrap_err());

        let mut usecase = TargetPersonUsecase { dao };

        let id: PersonId = 42;
        let result = usecase.find(id).run(&mut ());

        assert!(result.is_err());
        assert_eq!(result.err().unwrap(), expected);
    }

    #[test]
    fn test_entry_and_verify_insert_error() {
        let dao = StubPersonDao {
            insert_result: Err(DaoError::InsertError("valid dao".to_string())),
            fetch_result: Ok(None),    // 使わない
            select_result: Ok(vec![]), // 使わない
            delete_result: Ok(()),     // 使わない
        };
        let expected =
            UsecaseError::EntryAndVerifyPersonFailed(dao.insert_result.clone().unwrap_err());

        let mut usecase = TargetPersonUsecase { dao };

        let person = Person::new("Alice", date(2012, 11, 2), None, None);
        let result = usecase.entry_and_verify(person).run(&mut ());

        assert!(result.is_err());
        assert_eq!(result.err().unwrap(), expected);
    }

    #[test]
    fn test_entry_and_verify_fetch_error() {
        let dao = StubPersonDao {
            insert_result: Ok(42),
            fetch_result: Err(DaoError::SelectError("valid dao".to_string())),
            select_result: Ok(vec![]), // 使わない
            delete_result: Ok(()),     // 使わない
        };
        let expected =
            UsecaseError::EntryAndVerifyPersonFailed(dao.fetch_result.clone().unwrap_err());

        let mut usecase = TargetPersonUsecase { dao };

        let person = Person::new("Alice", date(2012, 11, 2), None, None);
        let result = usecase.entry_and_verify(person).run(&mut ());

        assert!(result.is_err());
        assert_eq!(result.err().unwrap(), expected);
    }

    #[test]
    fn test_collect() {
        let dao = StubPersonDao {
            insert_result: Ok(42),  // 使わない
            fetch_result: Ok(None), // 使わない
            select_result: Err(DaoError::SelectError("valid dao".to_string())),
            delete_result: Ok(()), // 使わない
        };
        let expected = UsecaseError::CollectPersonFailed(dao.select_result.clone().unwrap_err());

        let mut usecase = TargetPersonUsecase { dao };

        let result = usecase.collect().run(&mut ());

        assert!(result.is_err());
        assert_eq!(result.err().unwrap(), expected);
    }
    #[test]
    fn test_remove() {
        let dao = StubPersonDao {
            insert_result: Ok(42),     // 使わない
            fetch_result: Ok(None),    // 使わない
            select_result: Ok(vec![]), // 使わない
            delete_result: Err(DaoError::DeleteError("valid dao".to_string())),
        };
        let expected = UsecaseError::RemovePersonFailed(dao.delete_result.clone().unwrap_err());

        let mut usecase = TargetPersonUsecase { dao };

        let id: PersonId = 42;
        let result = usecase.remove(id).run(&mut ());

        assert!(result.is_err());
        assert_eq!(result.err().unwrap(), expected);
    }
}
