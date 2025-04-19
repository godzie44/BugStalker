"use strict";(self.webpackChunkwebsite=self.webpackChunkwebsite||[]).push([[930],{3752:(e,r,n)=>{n.d(r,{A:()=>i});var a=n(6540),s=n(4848);function i(e){let{src:r,options:n={}}=e;const i=(0,a.useRef)(null),[t,d]=(0,a.useState)(!1);return(0,a.useEffect)((()=>{if("undefined"==typeof window)return;const e="asciinema-player-css";if(!document.getElementById(e)){const r=document.createElement("link");r.id=e,r.rel="stylesheet",r.href="https://cdn.jsdelivr.net/npm/@asciinema/player@3.0.0/dist/themes/asciinema-player.css",document.head.appendChild(r)}const a="asciinema-player-script";let s=document.getElementById(a);s||(s=document.createElement("script"),s.id=a,s.src="https://cdn.jsdelivr.net/npm/@asciinema/player@3.0.0/dist/asciinema-player.min.js",s.async=!0,document.body.appendChild(s));const t=()=>{if(!window.AsciinemaPlayer)return void console.error("AsciinemaPlayer not available");const e=r.startsWith("http")?r:`${window.location.origin}${r}`;try{window.AsciinemaPlayer.create(e,i.current,{cols:120,rows:24,autoPlay:!0,fit:"width",...n}),d(!0)}catch(a){console.error("Player initialization failed:",a)}};return window.AsciinemaPlayer?t():s.onload=t,()=>{i.current&&(i.current.innerHTML="")}}),[r,n]),(0,s.jsx)("div",{ref:i,style:{minHeight:"300px",backgroundColor:t?"transparent":"#f5f5f5",borderRadius:"4px",margin:"20px 0",position:"relative"},children:!t&&(0,s.jsx)("div",{style:{position:"absolute",top:"50%",left:"50%",transform:"translate(-50%, -50%)",color:"#666"},children:"Loading player..."})})}},5363:(e,r,n)=>{n.d(r,{A:()=>i});n(6540);var a=n(9136),s=n(4848);function i(e){let{children:r,fallback:n}=e;return(0,a.A)()?(0,s.jsx)(s.Fragment,{children:r?.()}):n??null}},5533:(e,r,n)=>{n.r(r),n.d(r,{assets:()=>o,contentTitle:()=>l,default:()=>u,frontMatter:()=>c,metadata:()=>a,toc:()=>h});const a=JSON.parse('{"id":"capabilities/examine_data/print","title":"Print variables and arguments","description":"Now, let\u2019s see how you can observe data in a debugged program.","source":"@site/docs/capabilities/examine_data/print.mdx","sourceDirName":"capabilities/examine_data","slug":"/capabilities/examine_data/print","permalink":"/BugStalker/docs/capabilities/examine_data/print","draft":false,"unlisted":false,"editUrl":"https://github.com/facebook/docusaurus/tree/main/packages/create-docusaurus/templates/shared/docs/capabilities/examine_data/print.mdx","tags":[],"version":"current","sidebarPosition":1,"frontMatter":{"sidebar_position":1},"sidebar":"tutorialSidebar","previous":{"title":"Examine data","permalink":"/BugStalker/docs/category/examine-data"},"next":{"title":"Working with raw memory","permalink":"/BugStalker/docs/capabilities/examine_data/memory"}}');var s=n(4848),i=n(8453),t=n(5363),d=n(3752);const c={sidebar_position:1},l="Print variables and arguments",o={},h=[{value:"<code>var</code> and <code>vard</code> commands",id:"var-and-vard-commands",level:2},{value:"<code>arg</code> and <code>argd</code> commands",id:"arg-and-argd-commands",level:2},{value:"DQE",id:"dqe",level:2},{value:"Usage example",id:"usage-example",level:2}];function m(e){const r={code:"code",h1:"h1",h2:"h2",header:"header",li:"li",p:"p",pre:"pre",ul:"ul",...(0,i.R)(),...e.components};return(0,s.jsxs)(s.Fragment,{children:[(0,s.jsx)(r.header,{children:(0,s.jsx)(r.h1,{id:"print-variables-and-arguments",children:"Print variables and arguments"})}),"\n",(0,s.jsx)(r.p,{children:"Now, let\u2019s see how you can observe data in a debugged program."}),"\n",(0,s.jsxs)(r.h2,{id:"var-and-vard-commands",children:[(0,s.jsx)(r.code,{children:"var"})," and ",(0,s.jsx)(r.code,{children:"vard"})," commands"]}),"\n",(0,s.jsx)(r.p,{children:"To observe local and global variables, use these commands:"}),"\n",(0,s.jsxs)(r.ul,{children:["\n",(0,s.jsxs)(r.li,{children:[(0,s.jsx)(r.code,{children:"var ( <DQE> | locals )"})," - prints local and global variables."]}),"\n",(0,s.jsxs)(r.li,{children:[(0,s.jsx)(r.code,{children:"vard ( <DQE> | locals )"})," - same as the ",(0,s.jsx)(r.code,{children:"var"})," command, but uses the ",(0,s.jsx)(r.code,{children:"Debug"})," trait for rendering"]}),"\n"]}),"\n",(0,s.jsxs)(r.h2,{id:"arg-and-argd-commands",children:[(0,s.jsx)(r.code,{children:"arg"})," and ",(0,s.jsx)(r.code,{children:"argd"})," commands"]}),"\n",(0,s.jsxs)(r.ul,{children:["\n",(0,s.jsxs)(r.li,{children:[(0,s.jsx)(r.code,{children:"arg ( <DQE> | all )"})," - prints a function's arguments"]}),"\n",(0,s.jsxs)(r.li,{children:[(0,s.jsx)(r.code,{children:"argd ( <DQE> | all )"})," - same as the ",(0,s.jsx)(r.code,{children:"arg"})," command, but uses the ",(0,s.jsx)(r.code,{children:"Debug"})," trait for rendering"]}),"\n"]}),"\n",(0,s.jsx)(r.h2,{id:"dqe",children:"DQE"}),"\n",(0,s.jsx)(r.p,{children:"BugStalker has a special syntax for exploring program data, called Data Query Expression (DQE).\nYou can dereference references, access structure fields, slice arrays, or get elements from vectors by their index (and much more!)."}),"\n",(0,s.jsx)(r.p,{children:"Operators available in expression:"}),"\n",(0,s.jsxs)(r.ul,{children:["\n",(0,s.jsxs)(r.li,{children:["select a variable by its name (e.g., ",(0,s.jsx)(r.code,{children:"var a"}),")"]}),"\n",(0,s.jsxs)(r.li,{children:["dereference pointers, references, or smart pointers (e.g., ",(0,s.jsx)(r.code,{children:"var *ref_to_a"}),")"]}),"\n",(0,s.jsxs)(r.li,{children:["access a structure field (e.g., ",(0,s.jsx)(r.code,{children:"var some_struct.some_field"}),")"]}),"\n",(0,s.jsxs)(r.li,{children:["access an element by index or key from arrays, slices, vectors, or hashmaps (e.g., ",(0,s.jsx)(r.code,{children:"var arr[1]"})," or even ",(0,s.jsx)(r.code,{children:"var hm[{a: 1, b: 2}]"}),")"]}),"\n",(0,s.jsxs)(r.li,{children:["slice arrays, vectors, or slices (e.g., ",(0,s.jsx)(r.code,{children:"var some_vector[1..3]"})," or ",(0,s.jsx)(r.code,{children:"var some_vector[1..]"}),")"]}),"\n",(0,s.jsxs)(r.li,{children:["cast a constant address to a pointer of a specific type (e.g., ",(0,s.jsx)(r.code,{children:"var (*mut SomeType)0x123AABCD"}),")"]}),"\n",(0,s.jsxs)(r.li,{children:["take an address (e.g.,  ",(0,s.jsx)(r.code,{children:"var &some_struct.some_field"}),")"]}),"\n",(0,s.jsxs)(r.li,{children:["show a canonical representation (e.g., display a vector header instead of vector data: ",(0,s.jsx)(r.code,{children:"var ~myvec"}),")"]}),"\n",(0,s.jsx)(r.li,{children:"use parentheses to control operator execution order"}),"\n"]}),"\n",(0,s.jsx)(r.p,{children:"Writing expressions is simple, and you can do it right now! Here are some examples:"}),"\n",(0,s.jsxs)(r.ul,{children:["\n",(0,s.jsxs)(r.li,{children:[(0,s.jsx)(r.code,{children:"var *some_variable"})," - dereference and print value of ",(0,s.jsx)(r.code,{children:"some_variable"})]}),"\n",(0,s.jsxs)(r.li,{children:[(0,s.jsx)(r.code,{children:"var hm[{a: 1, b: *}]"})," - print the value from a hashmap corresponding to the key. The literal ",(0,s.jsx)(r.code,{children:"{a: 1, b: *}"})," matches any structure where field a equals 1 and field b can be any value"]}),"\n",(0,s.jsxs)(r.li,{children:[(0,s.jsx)(r.code,{children:"var some_array[0][2..5]"})," - print three elements, starting from index 2 of the first element in ",(0,s.jsx)(r.code,{children:"some_array"})]}),"\n",(0,s.jsxs)(r.li,{children:[(0,s.jsx)(r.code,{children:"var *some_array[0]"})," - print dereferenced value of ",(0,s.jsx)(r.code,{children:"some_array[0]"})]}),"\n",(0,s.jsxs)(r.li,{children:[(0,s.jsx)(r.code,{children:"var &some_array[0]"})," - print address of ",(0,s.jsx)(r.code,{children:"some_array[0]"})]}),"\n",(0,s.jsxs)(r.li,{children:[(0,s.jsx)(r.code,{children:"var (~some_vec).len"})," - print len field from the vector header"]}),"\n",(0,s.jsxs)(r.li,{children:[(0,s.jsx)(r.code,{children:"var (*some_array)[0]"})," - print the first element of ",(0,s.jsx)(r.code,{children:"*some_array"})]}),"\n",(0,s.jsxs)(r.li,{children:[(0,s.jsx)(r.code,{children:"var *(*(var1.field1)).field2[1][2]"})," - print the dereferenced value of element at index 2 in\nelement at index 1 of field ",(0,s.jsx)(r.code,{children:"field2"})," in dereferenced value of field ",(0,s.jsx)(r.code,{children:"field1"})," in variable ",(0,s.jsx)(r.code,{children:"var1"})," :)"]}),"\n"]}),"\n",(0,s.jsx)(r.h2,{id:"usage-example",children:"Usage example"}),"\n",(0,s.jsx)(r.p,{children:"Consider this Rust function:"}),"\n",(0,s.jsx)(r.pre,{children:(0,s.jsx)(r.code,{className:"language-rust",children:"fn my_func(arg1: &str, arg2: i32) {\n    let a = arg2;\n    let ref_a = &arg2;\n    let ref_ref_a = &arg2;\n\n    #[derive(Hash, PartialEq, Eq, Debug)]\n    struct Foo<'a> {\n        bar: &'a str,\n        baz: Vec<i32>,\n    }\n    let foo = Foo {\n        bar: arg1,\n        baz: vec![1, 2],\n    };\n\n    let hm1 = HashMap::from([(foo, 1)]);\n\n    let nop = Option::<u8>::None;\n}\n"})}),"\n",(0,s.jsx)(r.p,{children:"Let\u2019s observe the variables and arguments:"}),"\n","\n",(0,s.jsx)(t.A,{children:()=>(0,s.jsx)(d.A,{src:"/BugStalker/casts/print.cast"})})]})}function u(e={}){const{wrapper:r}={...(0,i.R)(),...e.components};return r?(0,s.jsx)(r,{...e,children:(0,s.jsx)(m,{...e})}):m(e)}},8453:(e,r,n)=>{n.d(r,{R:()=>t,x:()=>d});var a=n(6540);const s={},i=a.createContext(s);function t(e){const r=a.useContext(i);return a.useMemo((function(){return"function"==typeof e?e(r):{...r,...e}}),[r,e])}function d(e){let r;return r=e.disableParentContext?"function"==typeof e.components?e.components(s):e.components||s:t(e.components),a.createElement(i.Provider,{value:r},e.children)}}}]);