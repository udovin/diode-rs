use diode_base::defer;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

#[test]
fn test_defer_basic_functionality() {
    let executed = Arc::new(Mutex::new(false));
    let executed_clone = executed.clone();

    {
        let _defer = defer(move || {
            *executed_clone.lock().unwrap() = true;
        });

        // At this point, the defer should not have executed yet
        assert!(!*executed.lock().unwrap());
    } // defer should execute here when _defer goes out of scope

    // Now the defer should have executed
    assert!(*executed.lock().unwrap());
}

#[test]
fn test_defer_macro_basic() {
    let executed = Arc::new(Mutex::new(false));
    let executed_clone = executed.clone();

    {
        defer!({
            *executed_clone.lock().unwrap() = true;
        });

        // At this point, the defer should not have executed yet
        assert!(!*executed.lock().unwrap());
    } // defer should execute here

    // Now the defer should have executed
    assert!(*executed.lock().unwrap());
}

#[test]
fn test_defer_macro_single_expression() {
    let counter = Arc::new(Mutex::new(0));
    let counter_clone = counter.clone();

    {
        defer!(*counter_clone.lock().unwrap() += 1);
        assert_eq!(*counter.lock().unwrap(), 0);
    }

    assert_eq!(*counter.lock().unwrap(), 1);
}

#[test]
fn test_multiple_defers_lifo_order() {
    let execution_order = Arc::new(Mutex::new(Vec::new()));

    {
        let order1 = execution_order.clone();
        let order2 = execution_order.clone();
        let order3 = execution_order.clone();

        let _defer1 = defer(move || {
            order1.lock().unwrap().push(1);
        });

        let _defer2 = defer(move || {
            order2.lock().unwrap().push(2);
        });

        let _defer3 = defer(move || {
            order3.lock().unwrap().push(3);
        });

        // Nothing executed yet
        assert!(execution_order.lock().unwrap().is_empty());
    } // All defers execute here in reverse order (LIFO)

    // Should execute in reverse order: 3, 2, 1
    assert_eq!(*execution_order.lock().unwrap(), vec![3, 2, 1]);
}

#[test]
fn test_defer_in_function_scope() {
    fn test_function() -> i32 {
        let counter = Rc::new(RefCell::new(0));
        let counter_clone = counter.clone();

        let _defer = defer(move || {
            *counter_clone.borrow_mut() += 10;
        });

        *counter.borrow_mut() += 5;

        // Defer hasn't executed yet
        assert_eq!(*counter.borrow(), 5);

        *counter.borrow()
    } // defer executes here before function returns

    let result = test_function();
    // The function returned 5, but defer would have added 10 after that
    assert_eq!(result, 5);
}

#[test]
fn test_defer_with_mutable_reference() {
    let mut counter = 0;

    {
        let counter_ptr = &mut counter as *mut i32;
        let _defer = defer(move || unsafe {
            *counter_ptr += 1;
        });

        counter += 5;
        assert_eq!(counter, 5);
    }

    assert_eq!(counter, 6);
}

#[test]
fn test_defer_with_string_operations() {
    let result = Arc::new(Mutex::new(String::new()));
    let result_clone = result.clone();

    {
        let _defer = defer(move || {
            result_clone.lock().unwrap().push_str("-end");
        });

        result.lock().unwrap().push_str("start");
        assert_eq!(*result.lock().unwrap(), "start");
    }

    assert_eq!(*result.lock().unwrap(), "start-end");
}

#[test]
fn test_defer_explicit_drop() {
    let executed = Arc::new(Mutex::new(false));
    let executed_clone = executed.clone();

    let defer_guard = defer(move || {
        *executed_clone.lock().unwrap() = true;
    });

    // Should not be executed yet
    assert!(!*executed.lock().unwrap());

    // Explicitly drop the defer guard
    drop(defer_guard);

    // Should be executed now
    assert!(*executed.lock().unwrap());
}

#[test]
fn test_defer_nested_scopes() {
    let execution_order = Arc::new(Mutex::new(Vec::new()));

    {
        let order1 = execution_order.clone();
        let _defer_outer = defer(move || {
            order1.lock().unwrap().push("outer");
        });

        {
            let order2 = execution_order.clone();
            let _defer_inner = defer(move || {
                order2.lock().unwrap().push("inner");
            });

            assert!(execution_order.lock().unwrap().is_empty());
        } // inner defer executes here

        // Only inner should have executed
        assert_eq!(*execution_order.lock().unwrap(), vec!["inner"]);
    } // outer defer executes here

    assert_eq!(*execution_order.lock().unwrap(), vec!["inner", "outer"]);
}

#[test]
fn test_defer_with_complex_closure() {
    let results = Arc::new(Mutex::new(Vec::new()));

    {
        let results_clone = results.clone();
        let _defer = defer(move || {
            let mut guard = results_clone.lock().unwrap();
            for i in 1..=3 {
                guard.push(i * i);
            }
        });

        assert!(results.lock().unwrap().is_empty());
    }

    assert_eq!(*results.lock().unwrap(), vec![1, 4, 9]);
}

#[test]
fn test_defer_macro_with_multiple_statements() {
    let values = Arc::new(Mutex::new(Vec::new()));
    let values_clone = values.clone();

    {
        defer!({
            let mut guard = values_clone.lock().unwrap();
            guard.push(1);
            guard.push(2);
            guard.push(3);
        });

        assert!(values.lock().unwrap().is_empty());
    }

    assert_eq!(*values.lock().unwrap(), vec![1, 2, 3]);
}

#[test]
fn test_defer_multiple_times_same_scope() {
    let counter = Arc::new(Mutex::new(0));

    {
        let _defer0 = {
            let counter_clone = counter.clone();
            defer(move || {
                *counter_clone.lock().unwrap() += 0;
            })
        };
        let _defer1 = {
            let counter_clone = counter.clone();
            defer(move || {
                *counter_clone.lock().unwrap() += 1;
            })
        };
        let _defer2 = {
            let counter_clone = counter.clone();
            defer(move || {
                *counter_clone.lock().unwrap() += 2;
            })
        };
        let _defer3 = {
            let counter_clone = counter.clone();
            defer(move || {
                *counter_clone.lock().unwrap() += 3;
            })
        };
        let _defer4 = {
            let counter_clone = counter.clone();
            defer(move || {
                *counter_clone.lock().unwrap() += 4;
            })
        };

        assert_eq!(*counter.lock().unwrap(), 0);
    }

    // Should sum 0 + 1 + 2 + 3 + 4 = 10 (but in reverse order due to LIFO)
    assert_eq!(*counter.lock().unwrap(), 10);
}
