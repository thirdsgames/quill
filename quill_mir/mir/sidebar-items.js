initSidebarItems({"enum":[["DefinitionBodyM",""],["LocalVariableName","A local variable is a value which can be operated on by functions and expressions. Other objects, such as symbols in global scope, must be instanced as local variables before being operated on. This allows the borrow checker and the code translator to better understand the control flow and data flow."],["PlaceSegment",""],["Rvalue","Represents the use of a value that we can feed into an expression or function. We can only read from (not write to) an rvalue."],["StatementKind",""],["TerminatorKind",""]],"struct":[["ArgumentIndex",""],["BasicBlock","A basic block is a block of code that can be executed, and may manipulate values. Control flow is entirely linear inside a basic block. After this basic block, we may branch to one of several places."],["BasicBlockId",""],["ControlFlowGraph",""],["DefinitionM","A definition for a symbol, i.e. a function or constant. The function’s type is `arg_types -> return_type`. For example, if we defined a function"],["LocalVariableId",""],["LocalVariableInfo","Information about a local variable, either explicitly or implicitly defined."],["Place","A place in memory that we can read from and write to."],["Statement",""],["Terminator",""]]});