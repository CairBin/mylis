//! MIR v0 的 JSON 编解码：只为 `.mylib` 的反序列化服务，不改变 MIR 语义。

use crate::ast::{BinOp, Builtin, Pattern, Res, Type, UnOp};
use crate::json::{self, Value};
use crate::mir::*;

fn o(kv: Vec<(&str, Value)>) -> Value {
    Value::Obj(kv.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}
fn a(v: Vec<Value>) -> Value { Value::Arr(v) }
fn n(v: usize) -> Value { Value::Num(v as f64) }
fn i(v: i64) -> Value { Value::Num(v as f64) }
fn f(v: f64) -> Value { Value::Num(v) }
fn s(v: &str) -> Value { Value::Str(v.to_string()) }
fn b(v: bool) -> Value { Value::Bool(v) }
fn null() -> Value { Value::Null }
fn opt<T>(v: Option<T>, fnv: impl Fn(T) -> Value) -> Value {
    match v { Some(x) => fnv(x), None => null() }
}
fn as_n(v: &Value) -> Option<usize> { v.as_i64().map(|x| x as usize) }
fn as_i(v: &Value) -> Option<i64> { v.as_i64() }
fn as_b(v: &Value) -> Option<bool> { match v { Value::Bool(x) => Some(*x), _ => None } }
fn as_s(v: &Value) -> Option<&str> { v.as_str() }
fn as_f(v: &Value) -> Option<f64> { match v { Value::Num(x) => Some(*x), _ => None } }
fn obj(v: &Value, k: &str) -> Option<Value> { v.get(k).cloned() }
fn arr(v: &Value) -> Option<&Vec<Value>> { v.as_arr() }
fn tag(v: &Value) -> Option<&str> { v.get("k").and_then(|x| x.as_str()) }

fn ty(t: &Type) -> Value {
    use Type::*;
    match t {
        Void => o(vec![("k", s("void"))]), I8 => o(vec![("k", s("i8"))]),
        I16 => o(vec![("k", s("i16"))]), I32 => o(vec![("k", s("i32"))]),
        I64 => o(vec![("k", s("i64"))]), U8 => o(vec![("k", s("u8"))]),
        U16 => o(vec![("k", s("u16"))]), U32 => o(vec![("k", s("u32"))]),
        U64 => o(vec![("k", s("u64"))]), F32 => o(vec![("k", s("f32"))]),
        F64 => o(vec![("k", s("f64"))]), Bool => o(vec![("k", s("bool"))]),
        Str => o(vec![("k", s("str"))]), Infer => o(vec![("k", s("infer"))]),
        Named(x) => o(vec![("k", s("named")), ("v", s(x))]),
        Generic(x, _) => o(vec![("k", s("generic")), ("v", s(x))]),
        Adt { base, args } => o(vec![("k", s("adt")), ("id", n(*base)), ("args", Value::Arr(args.iter().map(ty).collect()))]),
        Param(x) => o(vec![("k", s("param")), ("v", s(x))]),
        Ptr(x) => o(vec![("k", s("ptr")), ("v", ty(x))]),
        Array(x) => o(vec![("k", s("array")), ("v", ty(x))]),
        Option(x) => o(vec![("k", s("option")), ("v", ty(x))]),
        Result(x, y) => o(vec![("k", s("result")), ("v", ty(x)), ("w", ty(y))]),
        Class(x) => o(vec![("k", s("class")), ("id", n(*x))]),
        Iface(x) => o(vec![("k", s("iface")), ("id", n(*x))]),
        Enum(x) => o(vec![("k", s("enum")), ("id", n(*x))]),
    }
}

fn ty_from(v: &Value) -> Option<Type> {
    match tag(v)? {
        "void" => Some(Type::Void), "i8" => Some(Type::I8), "i16" => Some(Type::I16),
        "i32" => Some(Type::I32), "i64" => Some(Type::I64), "u8" => Some(Type::U8),
        "u16" => Some(Type::U16), "u32" => Some(Type::U32), "u64" => Some(Type::U64),
        "f32" => Some(Type::F32), "f64" => Some(Type::F64), "bool" => Some(Type::Bool),
        "str" => Some(Type::Str), "infer" => Some(Type::Infer),
        "named" => Some(Type::Named(as_s(&obj(v, "v")?)?.to_string())),
        "adt" => Some(Type::Adt {
            base: as_n(&obj(v, "id")?)?,
            args: arr(&obj(v, "args")?).into_iter().flatten().filter_map(ty_from).collect(),
        }),
        "param" => Some(Type::Param(as_s(&obj(v, "v")?)?.to_string())),
        "ptr" => Some(Type::Ptr(Box::new(ty_from(&obj(v, "v")?)?))),
        "array" => Some(Type::Array(Box::new(ty_from(&obj(v, "v")?)?))),
        "option" => Some(Type::Option(Box::new(ty_from(&obj(v, "v")?)?))),
        "result" => Some(Type::Result(Box::new(ty_from(&obj(v, "v")?)?), Box::new(ty_from(&obj(v, "w")?)?))),
        "class" => Some(Type::Class(as_n(&obj(v, "id")?)?)),
        "iface" => Some(Type::Iface(as_n(&obj(v, "id")?)?)),
        "enum" => Some(Type::Enum(as_n(&obj(v, "id")?)?)),
        _ => None,
    }
}

fn op(x: MirOperand) -> Value {
    match x { MirOperand::Value(v) => o(vec![("k", s("v")), ("id", n(v as usize))]), MirOperand::Local(v) => o(vec![("k", s("l")), ("id", n(v as usize))]) }
}
fn op_from(v: &Value) -> Option<MirOperand> {
    match tag(v)? {
        "v" => Some(MirOperand::Value(as_n(&obj(v, "id")?)? as u32)),
        "l" => Some(MirOperand::Local(as_n(&obj(v, "id")?)? as u32)),
        _ => None,
    }
}

fn const_v(c: &MirConst) -> Value {
    match c {
        MirConst::Int(x) => o(vec![("k", s("int")), ("v", i(*x))]),
        MirConst::IntU64(x) => o(vec![("k", s("intu64")), ("v", s(&x.to_string()))]),
        MirConst::Float(x) => o(vec![("k", s("float")), ("v", f(*x))]),
        MirConst::Bool(x) => o(vec![("k", s("bool")), ("v", b(*x))]),
        MirConst::Str(x) => o(vec![("k", s("str")), ("v", s(x))]),
        MirConst::Null => o(vec![("k", s("null"))]),
    }
}
fn const_from(v: &Value) -> Option<MirConst> {
    match tag(v)? {
        "int" => Some(MirConst::Int(as_i(&obj(v, "v")?)?)),
        "intu64" => Some(MirConst::IntU64(as_s(&obj(v, "v")?)?.parse::<u64>().ok()?)),
        "float" => Some(MirConst::Float(as_f(&obj(v, "v")?)?)),
        "bool" => Some(MirConst::Bool(as_b(&obj(v, "v")?)?)),
        "str" => Some(MirConst::Str(as_s(&obj(v, "v")?)?.to_string())),
        "null" => Some(MirConst::Null),
        _ => None,
    }
}

fn bin_name(x: BinOp) -> &'static str { match x {
    BinOp::Add=>"add", BinOp::Sub=>"sub", BinOp::Mul=>"mul", BinOp::Div=>"div", BinOp::Rem=>"rem",
    BinOp::Eq=>"eq", BinOp::Ne=>"ne", BinOp::Lt=>"lt", BinOp::Le=>"le", BinOp::Gt=>"gt",
    BinOp::Ge=>"ge", BinOp::AndAnd=>"and", BinOp::OrOr=>"or", BinOp::BitAnd=>"bitand",
    BinOp::BitOr=>"bitor", BinOp::BitXor=>"bitxor", BinOp::Shl=>"shl", BinOp::Shr=>"shr",
}}
fn bin_from(x: &str) -> Option<BinOp> { Some(match x {
    "add"=>BinOp::Add,"sub"=>BinOp::Sub,"mul"=>BinOp::Mul,"div"=>BinOp::Div,"rem"=>BinOp::Rem,
    "eq"=>BinOp::Eq,"ne"=>BinOp::Ne,"lt"=>BinOp::Lt,"le"=>BinOp::Le,"gt"=>BinOp::Gt,"ge"=>BinOp::Ge,
    "and"=>BinOp::AndAnd,"or"=>BinOp::OrOr,"bitand"=>BinOp::BitAnd,"bitor"=>BinOp::BitOr,
    "bitxor"=>BinOp::BitXor,"shl"=>BinOp::Shl,"shr"=>BinOp::Shr,_=>return None,
}) }
fn un_name(x: UnOp) -> &'static str { match x { UnOp::Neg=>"neg",UnOp::Not=>"not",UnOp::BitNot=>"bitnot",UnOp::Deref=>"deref",UnOp::AddrOf=>"addr" } }
fn un_from(x: &str) -> Option<UnOp> { Some(match x { "neg"=>UnOp::Neg,"not"=>UnOp::Not,"bitnot"=>UnOp::BitNot,"deref"=>UnOp::Deref,"addr"=>UnOp::AddrOf,_=>return None }) }

fn builtin_name(x: Builtin) -> &'static str { match x {
    Builtin::ResIsOk=>"res_is_ok",Builtin::ResIsErr=>"res_is_err",Builtin::ResUnwrap=>"res_unwrap",
    Builtin::ResUnwrapErr=>"res_unwrap_err",Builtin::ResUnwrapOr=>"res_unwrap_or",
    Builtin::OptIsSome=>"opt_is_some",Builtin::OptIsNone=>"opt_is_none",Builtin::OptUnwrap=>"opt_unwrap",
    Builtin::OptUnwrapOr=>"opt_unwrap_or",Builtin::StrLen=>"str_len",Builtin::ArrLen=>"arr_len",
    Builtin::ValueEq=>"value_eq",Builtin::ValueCmp=>"value_cmp",Builtin::ValueHash=>"value_hash",
}}
fn builtin_from(x: &str) -> Option<Builtin> { Some(match x {
    "res_is_ok"=>Builtin::ResIsOk,"res_is_err"=>Builtin::ResIsErr,"res_unwrap"=>Builtin::ResUnwrap,
    "res_unwrap_err"=>Builtin::ResUnwrapErr,"res_unwrap_or"=>Builtin::ResUnwrapOr,
    "opt_is_some"=>Builtin::OptIsSome,"opt_is_none"=>Builtin::OptIsNone,"opt_unwrap"=>Builtin::OptUnwrap,
    "opt_unwrap_or"=>Builtin::OptUnwrapOr,"str_len"=>Builtin::StrLen,"arr_len"=>Builtin::ArrLen,
    "value_eq"=>Builtin::ValueEq,"value_cmp"=>Builtin::ValueCmp,"value_hash"=>Builtin::ValueHash,
    _=>return None,
}) }

fn res_v(r: &Res) -> Value { use Res::*; match r {
    None=>o(vec![("k",s("none"))]), Var=>o(vec![("k",s("var"))]),
    Fn(x)=>o(vec![("k",s("fn")),("id",n(*x))]), Method(x)=>o(vec![("k",s("method")),("id",n(*x))]),
    Field(c,f)=>o(vec![("k",s("field")),("c",n(*c)),("f",n(*f))]),
    Ctor(c)=>o(vec![("k",s("ctor")),("id",n(*c))]),
    IfaceCast(c,i)=>o(vec![("k",s("iface_cast")),("c",n(*c)),("i",n(*i))]),
    Variant(e,v)=>o(vec![("k",s("variant")),("e",n(*e)),("v",n(*v))]),
    StaticMethod(c,m)=>o(vec![("k",s("static_method")),("c",n(*c)),("m",n(*m))]),
    StaticField(c,f)=>o(vec![("k",s("static_field")),("c",n(*c)),("f",n(*f))]),
    Builtin(x)=>o(vec![("k",s("builtin")),("name",s(builtin_name(*x)))]),
    AdtCtor{base,args}=>o(vec![("k",s("adt_ctor")),("b",n(*base)),("args",Value::Arr(args.iter().map(ty).collect()))]),
    AdtFn{base,args}=>o(vec![("k",s("adt_fn")),("b",n(*base)),("args",Value::Arr(args.iter().map(ty).collect()))]),
    AdtMethod{base,method,class_args,method_args}=>o(vec![
        ("k",s("adt_method")),("b",n(*base)),("m",n(*method)),
        ("cargs",Value::Arr(class_args.iter().map(ty).collect())),
        ("margs",Value::Arr(method_args.iter().map(ty).collect())),
    ]),
    AdtStaticMethod{base,method,class_args,method_args}=>o(vec![
        ("k",s("adt_static")),("b",n(*base)),("m",n(*method)),
        ("cargs",Value::Arr(class_args.iter().map(ty).collect())),
        ("margs",Value::Arr(method_args.iter().map(ty).collect())),
    ]),
    AdtField{base,field,class_args}=>o(vec![("k",s("adt_inst_field")),("b",n(*base)),("f",n(*field)),("args",Value::Arr(class_args.iter().map(ty).collect()))]),
    AdtStaticField{base,field,class_args}=>o(vec![("k",s("adt_field")),("b",n(*base)),("f",n(*field)),("args",Value::Arr(class_args.iter().map(ty).collect()))]),
    AdtVariant{base,variant,args}=>o(vec![("k",s("adt_variant")),("b",n(*base)),("v",n(*variant)),("args",Value::Arr(args.iter().map(ty).collect()))]),
}}
fn res_from(v: &Value) -> Option<Res> { match tag(v)? {
    "none"=>Some(Res::None), "var"=>Some(Res::Var),
    "fn"=>Some(Res::Fn(as_n(&obj(v,"id")?)?)), "method"=>Some(Res::Method(as_n(&obj(v,"id")?)?)),
    "field"=>Some(Res::Field(as_n(&obj(v,"c")?)?,as_n(&obj(v,"f")?)?)),
    "ctor"=>Some(Res::Ctor(as_n(&obj(v,"id")?)?)),
    "iface_cast"=>Some(Res::IfaceCast(as_n(&obj(v,"c")?)?,as_n(&obj(v,"i")?)?)),
    "variant"=>Some(Res::Variant(as_n(&obj(v,"e")?)?,as_n(&obj(v,"v")?)?)),
    "static_method"=>Some(Res::StaticMethod(as_n(&obj(v,"c")?)?,as_n(&obj(v,"m")?)?)),
    "static_field"=>Some(Res::StaticField(as_n(&obj(v,"c")?)?,as_n(&obj(v,"f")?)?)),
    "builtin"=>Some(Res::Builtin(builtin_from(as_s(&obj(v,"name")?)?)?)),
    "adt_ctor"=>Some(Res::AdtCtor {
        base: as_n(&obj(v,"b")?)?,
        args: arr(&obj(v,"args")?)?.iter().map(ty_from).collect::<Option<Vec<_>>>()?,
    }),
    "adt_fn"=>Some(Res::AdtFn {
        base: as_n(&obj(v,"b")?)?,
        args: arr(&obj(v,"args")?)?.iter().map(ty_from).collect::<Option<Vec<_>>>()?,
    }),
    "adt_method"=>Some(Res::AdtMethod {
        base: as_n(&obj(v,"b")?)?,
        method: as_n(&obj(v,"m")?)?,
        class_args: arr(&obj(v,"cargs")?)?.iter().map(ty_from).collect::<Option<Vec<_>>>()?,
        method_args: arr(&obj(v,"margs")?)?.iter().map(ty_from).collect::<Option<Vec<_>>>()?,
    }),
    "adt_static"=>Some(Res::AdtStaticMethod {
        base: as_n(&obj(v,"b")?)?,
        method: as_n(&obj(v,"m")?)?,
        class_args: arr(&obj(v,"cargs")?)?.iter().map(ty_from).collect::<Option<Vec<_>>>()?,
        method_args: arr(&obj(v,"margs")?)?.iter().map(ty_from).collect::<Option<Vec<_>>>()?,
    }),
    "adt_inst_field"=>Some(Res::AdtField {
        base: as_n(&obj(v,"b")?)?,
        field: as_n(&obj(v,"f")?)?,
        class_args: arr(&obj(v,"args")?)?.iter().map(ty_from).collect::<Option<Vec<_>>>()?,
    }),
    "adt_field"=>Some(Res::AdtStaticField {
        base: as_n(&obj(v,"b")?)?,
        field: as_n(&obj(v,"f")?)?,
        class_args: arr(&obj(v,"args")?)?.iter().map(ty_from).collect::<Option<Vec<_>>>()?,
    }),
    "adt_variant"=>Some(Res::AdtVariant {
        base: as_n(&obj(v,"b")?)?,
        variant: as_n(&obj(v,"v")?)?,
        args: arr(&obj(v,"args")?)?.iter().map(ty_from).collect::<Option<Vec<_>>>()?,
    }),
    _=>None,
}}

fn pat_v(p: &Pattern) -> Value { use Pattern::*; match p {
    Wildcard=>o(vec![("k",s("wild"))]), Int(x)=>o(vec![("k",s("int")),("v",i(*x))]),
    IntU64(x)=>o(vec![("k",s("intu64")),("v",s(&x.to_string()))]),
    Float(x)=>o(vec![("k",s("float")),("v",f(*x))]), Str(x)=>o(vec![("k",s("str")),("v",s(x))]),
    Bool(x)=>o(vec![("k",s("bool")),("v",b(*x))]), Null=>o(vec![("k",s("null"))]),
    Ok(x)=>o(vec![("k",s("ok")),("b",opt(x.clone(),|y|s(&y)))]),
    Err(x)=>o(vec![("k",s("err")),("b",opt(x.clone(),|y|s(&y)))]),
    Some(x)=>o(vec![("k",s("some")),("b",opt(x.clone(),|y|s(&y)))]),
    None=>o(vec![("k",s("none"))]),
    Variant{ename,name,binds,enum_id,vidx}=>o(vec![
        ("k",s("variant")),("ename",s(ename)),("name",s(name)),
        ("binds",a(binds.iter().map(|x|opt(x.clone(),|y|s(&y))).collect())),
        ("enum_id",n(*enum_id)),("vidx",n(*vidx)),
    ]),
}}
fn pat_from(v: &Value) -> Option<Pattern> { match tag(v)? {
    "wild"=>Some(Pattern::Wildcard), "int"=>Some(Pattern::Int(as_i(&obj(v,"v")?)?)),
    "intu64"=>Some(Pattern::IntU64(as_s(&obj(v,"v")?)?.parse::<u64>().ok()?)),
    "float"=>Some(Pattern::Float(as_f(&obj(v,"v")?)?)), "str"=>Some(Pattern::Str(as_s(&obj(v,"v")?)?.to_string())),
    "bool"=>Some(Pattern::Bool(as_b(&obj(v,"v")?)?)), "null"=>Some(Pattern::Null),
    "ok"=>Some(Pattern::Ok(opt_str(&obj(v,"b")?)?)), "err"=>Some(Pattern::Err(opt_str(&obj(v,"b")?)?)),
    "some"=>Some(Pattern::Some(opt_str(&obj(v,"b")?)?)), "none"=>Some(Pattern::None),
    "variant"=>Some(Pattern::Variant{
        ename:as_s(&obj(v,"ename")?)?.to_string(), name:as_s(&obj(v,"name")?)?.to_string(),
        binds: arr(&obj(v,"binds")?)?.iter().map(opt_str).collect::<Option<Vec<_>>>()?,
        enum_id:as_n(&obj(v,"enum_id")?)?, vidx:as_n(&obj(v,"vidx")?)?,
    }),
    _=>None,
}}

fn opt_str(v: &Value) -> Option<Option<String>> { match v { Value::Null=>Some(None), _=>Some(Some(as_s(v)?.to_string())) } }

fn inst_v(i: &MirInst) -> Value {
    let kind = kind_v(&i.kind);
    o(vec![
        ("dst", opt(i.dst, |x| n(x as usize))), ("ty",ty(&i.ty)), ("res",res_v(&i.res)),
        ("coerce", opt(i.coerce, |x| o(vec![("c",n(x.0)),("i",n(x.1))]))),
        ("kind",kind), ("line",n(i.line as usize)),
    ])
}
fn inst_from(v: &Value) -> Option<MirInst> {
    Some(MirInst {
        dst: match obj(v,"dst")? { Value::Null=>None, x=>Some(as_n(&x)? as u32) },
        ty: ty_from(&obj(v,"ty")?)?,
        res: res_from(&obj(v,"res")?)?,
        coerce: match obj(v,"coerce")? { Value::Null=>None, x=>Some((as_n(&obj(&x,"c")?)?,as_n(&obj(&x,"i")?)?)) },
        kind: kind_from(&obj(v,"kind")?)?,
        line: as_n(&obj(v,"line")?)? as u32,
    })
}
fn args_v(x: &[MirOperand]) -> Value { a(x.iter().copied().map(op).collect()) }
fn args_from(v: &Value) -> Option<Vec<MirOperand>> { arr(v)?.iter().map(op_from).collect() }

fn kind_v(k: &MirInstKind) -> Value { use MirInstKind::*; match k {
    Const(x)=>o(vec![("k",s("const")),("v",const_v(x))]), Undef=>o(vec![("k",s("undef"))]),
    Copy(x)=>o(vec![("k",s("copy")),("v",op(*x))]),
    Store{local,value}=>o(vec![("k",s("store")),("l",n(*local as usize)),("v",op(*value))]),
    Bin(x,y,z)=>o(vec![("k",s("bin")),("op",s(bin_name(*x))),("a",op(*y)),("b",op(*z))]),
    Un(x,y)=>o(vec![("k",s("un")),("op",s(un_name(*x))),("v",op(*y))]),
    Call{callee,args}=>o(vec![("k",s("call")),("callee",s(callee)),("args",args_v(args))]),
    MethodCall{recv,name,args}=>o(vec![("k",s("method_call")),("recv",op(*recv)),("name",s(name)),("args",args_v(args))]),
    Field{base,name}=>o(vec![("k",s("field")),("base",op(*base)),("name",s(name))]),
    StoreField{base,name,value}=>o(vec![("k",s("store_field")),("base",op(*base)),("name",s(name)),("v",op(*value))]),
    Index{base,index}=>o(vec![("k",s("index")),("base",op(*base)),("i",op(*index))]),
    StoreIndex{base,index,value}=>o(vec![("k",s("store_index")),("base",op(*base)),("i",op(*index)),("v",op(*value))]),
    New{class,args}=>o(vec![("k",s("new")),("class",s(class)),("args",args_v(args))]),
    ArrayNew{elem,len}=>o(vec![("k",s("array_new")),("elem",s(elem)),("len",op(*len))]),
    ArrayLit{elem,values}=>o(vec![("k",s("array_lit")),("elem",s(elem)),("values",args_v(values))]),
    Cast{value,target,checked}=>o(vec![("k",s("cast")),("v",op(*value)),("target",ty(target)),("checked",b(*checked))]),
    Box{value,from}=>o(vec![("k",s("box")),("v",op(*value)),("from",ty(from))]),
    Try{value}=>o(vec![("k",s("try")),("v",op(*value))]),
    Interp{parts}=>o(vec![("k",s("interp")),("parts",a(parts.iter().map(interp_v).collect()))]),
    Global(x)=>o(vec![("k",s("global")),("v",s(x))]),
    StaticRef(x)=>o(vec![("k",s("static_ref")),("v",s(x))]),
    StoreStatic{name,value}=>o(vec![("k",s("store_static")),("name",s(name)),("v",op(*value))]),
    StorePtr{ptr,value}=>o(vec![("k",s("store_ptr")),("p",op(*ptr)),("v",op(*value))]),
    MatchTest{subject,pattern}=>o(vec![("k",s("match_test")),("v",op(*subject)),("p",pat_v(pattern))]),
    BindPattern{subject,pattern,locals}=>o(vec![("k",s("bind_pattern")),("v",op(*subject)),("p",pat_v(pattern)),("locals",a(locals.iter().map(|x|n(*x as usize)).collect()))]),
    Safepoint{roots}=>o(vec![("k",s("safepoint")),("roots",args_v(roots))]),
    Opaque(x)=>o(vec![("k",s("opaque")),("v",s(x))]),
}}
fn kind_from(v: &Value) -> Option<MirInstKind> { let g=|k:&str| obj(v,k); Some(match tag(v)? {
    "const"=>MirInstKind::Const(const_from(&g("v")?)?), "undef"=>MirInstKind::Undef,
    "copy"=>MirInstKind::Copy(op_from(&g("v")?)?),
    "store"=>MirInstKind::Store{local:as_n(&g("l")?)? as u32,value:op_from(&g("v")?)?},
    "bin"=>MirInstKind::Bin(bin_from(as_s(&g("op")?)?)?,op_from(&g("a")?)?,op_from(&g("b")?)?),
    "un"=>MirInstKind::Un(un_from(as_s(&g("op")?)?)?,op_from(&g("v")?)?),
    "call"=>MirInstKind::Call{callee:as_s(&g("callee")?)?.to_string(),args:args_from(&g("args")?)?},
    "method_call"=>MirInstKind::MethodCall{recv:op_from(&g("recv")?)?,name:as_s(&g("name")?)?.to_string(),args:args_from(&g("args")?)?},
    "field"=>MirInstKind::Field{base:op_from(&g("base")?)?,name:as_s(&g("name")?)?.to_string()},
    "store_field"=>MirInstKind::StoreField{base:op_from(&g("base")?)?,name:as_s(&g("name")?)?.to_string(),value:op_from(&g("v")?)?},
    "index"=>MirInstKind::Index{base:op_from(&g("base")?)?,index:op_from(&g("i")?)?},
    "store_index"=>MirInstKind::StoreIndex{base:op_from(&g("base")?)?,index:op_from(&g("i")?)?,value:op_from(&g("v")?)?},
    "new"=>MirInstKind::New{class:as_s(&g("class")?)?.to_string(),args:args_from(&g("args")?)?},
    "array_new"=>MirInstKind::ArrayNew{elem:as_s(&g("elem")?)?.to_string(),len:op_from(&g("len")?)?},
    "array_lit"=>MirInstKind::ArrayLit{elem:as_s(&g("elem")?)?.to_string(),values:args_from(&g("values")?)?},
    "cast"=>MirInstKind::Cast{value:op_from(&g("v")?)?,target:ty_from(&g("target")?)?,checked:as_b(&g("checked")?)?},
    "box"=>MirInstKind::Box{value:op_from(&g("v")?)?,from:ty_from(&g("from")?)?},
    "try"=>MirInstKind::Try{value:op_from(&g("v")?)?},
    "interp"=>MirInstKind::Interp{parts:arr(&g("parts")?)?.iter().map(interp_from).collect::<Option<Vec<_>>>()?},
    "global"=>MirInstKind::Global(as_s(&g("v")?)?.to_string()),
    "static_ref"=>MirInstKind::StaticRef(as_s(&g("v")?)?.to_string()),
    "store_static"=>MirInstKind::StoreStatic{name:as_s(&g("name")?)?.to_string(),value:op_from(&g("v")?)?},
    "store_ptr"=>MirInstKind::StorePtr{ptr:op_from(&g("p")?)?,value:op_from(&g("v")?)?},
    "match_test"=>MirInstKind::MatchTest{subject:op_from(&g("v")?)?,pattern:pat_from(&g("p")?)?},
    "bind_pattern"=>MirInstKind::BindPattern{subject:op_from(&g("v")?)?,pattern:pat_from(&g("p")?)?,locals:arr(&g("locals")?)?.iter().map(|x|as_n(x).map(|y|y as u32)).collect::<Option<Vec<_>>>()?},
    "safepoint"=>MirInstKind::Safepoint{roots:args_from(&g("roots")?)?},
    "opaque"=>MirInstKind::Opaque(as_s(&g("v")?)?.to_string()),
    _=>return None,
}) }
fn interp_v(p:&MirInterpPart)->Value{match p{MirInterpPart::Lit(x)=>o(vec![("k",s("lit")),("v",s(x))]),MirInterpPart::Value(x)=>o(vec![("k",s("val")),("v",op(*x))])}}
fn interp_from(v:&Value)->Option<MirInterpPart>{match tag(v)?{"lit"=>Some(MirInterpPart::Lit(as_s(&obj(v,"v")?)?.to_string())),"val"=>Some(MirInterpPart::Value(op_from(&obj(v,"v")?)?)),_=>None}}

fn term_v(t:&MirTerm)->Value{match t{
    MirTerm::Return(x)=>o(vec![("k",s("ret")),("v",opt(*x,op))]),
    MirTerm::Goto(x)=>o(vec![("k",s("goto")),("l",s(x))]),
    MirTerm::Branch{cond,then_label,else_label}=>o(vec![("k",s("br")),("v",op(*cond)),("t",s(then_label)),("e",s(else_label))]),
    MirTerm::Unreachable=>o(vec![("k",s("unreachable"))]),
}}
fn term_from(v:&Value)->Option<MirTerm>{Some(match tag(v)?{
    "ret"=>MirTerm::Return(match obj(v,"v")?{Value::Null=>None,x=>Some(op_from(&x)?)}),
    "goto"=>MirTerm::Goto(as_s(&obj(v,"l")?)?.to_string()),
    "br"=>MirTerm::Branch{cond:op_from(&obj(v,"v")?)?,then_label:as_s(&obj(v,"t")?)?.to_string(),else_label:as_s(&obj(v,"e")?)?.to_string()},
    "unreachable"=>MirTerm::Unreachable,_=>return None,
})}

fn local_v(l:&MirLocal)->Value{o(vec![("name",s(&l.name)),("ty",ty(&l.ty)),("mutable",b(l.mutable))])}
fn local_from(v:&Value)->Option<MirLocal>{Some(MirLocal{name:as_s(&obj(v,"name")?)?.to_string(),ty:ty_from(&obj(v,"ty")?)?,mutable:as_b(&obj(v,"mutable")?)?})}

fn func_v(f:&MirFunction)->Value{o(vec![
    ("name",s(&f.name)),
    ("decl_id",n(f.decl_id)),
    ("owner",opt(f.owner,n)),
    ("is_static",b(f.is_static)),
    ("is_ctor",b(f.is_ctor)),
    ("params",a(f.params.iter().map(local_v).collect())),
    ("locals",a(f.locals.iter().map(local_v).collect())),
    ("ret",ty(&f.ret)),
    ("blocks",a(f.blocks.iter().map(block_v).collect())),
])}
fn func_from(v:&Value)->Option<MirFunction>{Some(MirFunction{
    name:as_s(&obj(v,"name")?)?.to_string(),
    decl_id:as_n(&obj(v,"decl_id")?)?,
    owner:match obj(v,"owner")?{Value::Null=>None,x=>Some(as_n(&x)?)},
    is_static:as_b(&obj(v,"is_static")?)?,
    is_ctor:as_b(&obj(v,"is_ctor")?)?,
    params:arr(&obj(v,"params")?)?.iter().map(local_from).collect::<Option<Vec<_>>>()?,
    locals:arr(&obj(v,"locals")?)?.iter().map(local_from).collect::<Option<Vec<_>>>()?,
    ret:ty_from(&obj(v,"ret")?)?,
    blocks:arr(&obj(v,"blocks")?)?.iter().map(block_from).collect::<Option<Vec<_>>>()?,
})}
fn block_v(x:&MirBlock)->Value{o(vec![
    ("label",s(&x.label)),
    ("insts",a(x.insts.iter().map(inst_v).collect())),
    ("term",opt(x.term.clone(),|t|term_v(&t))),
])}
fn block_from(v:&Value)->Option<MirBlock>{Some(MirBlock{
    label:as_s(&obj(v,"label")?)?.to_string(),
    insts:arr(&obj(v,"insts")?)?.iter().map(inst_from).collect::<Option<Vec<_>>>()?,
    term:match obj(v,"term")?{Value::Null=>None,x=>Some(term_from(&x)?)},
})}

fn class_v(c:&MirClass)->Value{o(vec![
    ("name",s(&c.name)),
    ("decl_id",n(c.decl_id)),
    ("ctor",opt(c.ctor,n)),
    ("fields",a(c.fields.iter().map(|f| o(vec![
        ("name",s(&f.name)),
        ("ty",ty(&f.ty)),
        ("is_static",b(f.is_static)),
        ("is_pub",b(f.is_pub)),
        ("init",opt(f.init.clone(),|x|const_v(&x))),
    ])).collect())),
])}
fn class_from(v:&Value)->Option<MirClass>{Some(MirClass{
    name:as_s(&obj(v,"name")?)?.to_string(),
    decl_id:as_n(&obj(v,"decl_id")?)?,
    ctor:match obj(v,"ctor")?{Value::Null=>None,x=>Some(as_n(&x)?)},
    fields:arr(&obj(v,"fields")?)?.iter().map(|x| Some(MirField{
        name:as_s(&obj(x,"name")?)?.to_string(),
        ty:ty_from(&obj(x,"ty")?)?,
        is_static:as_b(&obj(x,"is_static")?)?,
        is_pub:as_b(&obj(x,"is_pub")?)?,
        init:match obj(x,"init")?{Value::Null=>None,y=>Some(const_from(&y)?)},
    })).collect::<Option<Vec<_>>>()?,
})}

fn enum_v(e:&MirEnum)->Value{o(vec![
    ("name",s(&e.name)),
    ("decl_id",n(e.decl_id)),
    ("variants",a(e.variants.iter().map(|v| o(vec![
        ("name",s(&v.name)),
        ("fields",a(v.fields.iter().map(ty).collect())),
    ])).collect())),
])}
fn enum_from(v:&Value)->Option<MirEnum>{Some(MirEnum{
    name:as_s(&obj(v,"name")?)?.to_string(),
    decl_id:as_n(&obj(v,"decl_id")?)?,
    variants:arr(&obj(v,"variants")?)?.iter().map(|x| Some(MirVariant{
        name:as_s(&obj(x,"name")?)?.to_string(),
        fields:arr(&obj(x,"fields")?)?.iter().map(ty_from).collect::<Option<Vec<_>>>()?,
    })).collect::<Option<Vec<_>>>()?,
})}

fn iface_v(i:&MirInterface)->Value{o(vec![
    ("name",s(&i.name)),
    ("decl_id",n(i.decl_id)),
    ("methods",a(i.methods.iter().map(|m| o(vec![
        ("name",s(&m.name)),
        ("decl_id",n(m.decl_id)),
        ("ret",ty(&m.ret)),
        ("params",a(m.params.iter().map(ty).collect())),
    ])).collect())),
])}
fn iface_from(v:&Value)->Option<MirInterface>{Some(MirInterface{
    name:as_s(&obj(v,"name")?)?.to_string(),
    decl_id:as_n(&obj(v,"decl_id")?)?,
    methods:arr(&obj(v,"methods")?)?.iter().map(|x| Some(MirIfaceMethod{
        name:as_s(&obj(x,"name")?)?.to_string(),
        decl_id:as_n(&obj(x,"decl_id")?)?,
        ret:ty_from(&obj(x,"ret")?)?,
        params:arr(&obj(x,"params")?)?.iter().map(ty_from).collect::<Option<Vec<_>>>()?,
    })).collect::<Option<Vec<_>>>()?,
})}

pub fn to_json(p:&MirProgram)->Value{o(vec![
    ("funcs",a(p.funcs.iter().map(func_v).collect())),
    ("classes",a(p.classes.iter().map(class_v).collect())),
    ("enums",a(p.enums.iter().map(enum_v).collect())),
    ("ifaces",a(p.ifaces.iter().map(iface_v).collect())),
])}
pub fn from_json(v:&Value)->Option<MirProgram>{Some(MirProgram{
    funcs:arr(&obj(v,"funcs")?)?.iter().map(func_from).collect::<Option<Vec<_>>>()?,
    classes:arr(&obj(v,"classes")?)?.iter().map(class_from).collect::<Option<Vec<_>>>()?,
    enums:arr(&obj(v,"enums")?)?.iter().map(enum_from).collect::<Option<Vec<_>>>()?,
    ifaces:arr(&obj(v,"ifaces")?)?.iter().map(iface_from).collect::<Option<Vec<_>>>()?,
})}
pub fn write(p:&MirProgram)->String{let mut s=String::new();json::write(&to_json(p),&mut s);s}
pub fn read(src:&str)->Option<MirProgram>{from_json(&json::parse(src)?)}

#[cfg(test)]
mod tests {
    use super::*;

    fn nested_adt() -> Type {
        Type::Adt {
            base: 7,
            args: vec![
                Type::Param("T".to_string()),
                Type::Adt { base: 9, args: vec![Type::I64] },
            ],
        }
    }

    #[test]
    fn adt_param_types_roundtrip() {
        let cases = vec![
            Type::Adt {
                base: 3,
                args: vec![Type::Param("A".to_string()), Type::Param("B".to_string())],
            },
            nested_adt(),
            Type::Option(Box::new(nested_adt())),
            Type::Result(Box::new(Type::Param("T".to_string())), Box::new(nested_adt())),
        ];
        for t in cases {
            let encoded = ty(&t);
            assert_eq!(ty_from(&encoded), Some(t));
        }
    }

    #[test]
    fn adt_res_roundtrip() {
        let t = nested_adt();
        let cases = vec![
            Res::AdtCtor { base: 7, args: vec![t.clone()] },
            Res::AdtFn { base: 8, args: vec![Type::Param("T".to_string()), t.clone()] },
            Res::AdtMethod {
                base: 7,
                method: 11,
                class_args: vec![t.clone()],
                method_args: vec![Type::Param("U".to_string())],
            },
            Res::AdtStaticMethod {
                base: 7,
                method: 12,
                class_args: vec![Type::Param("T".to_string())],
                method_args: vec![t.clone()],
            },
            Res::AdtField { base: 7, field: 1, class_args: vec![t.clone()] },
            Res::AdtStaticField { base: 7, field: 2, class_args: vec![t.clone()] },
            Res::AdtVariant { base: 13, variant: 1, args: vec![t.clone()] },
        ];
        for r in cases {
            let encoded = res_v(&r);
            assert_eq!(res_from(&encoded), Some(r));
        }
    }
}
