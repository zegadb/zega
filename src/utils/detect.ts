export function detect(str: any) {

    if (str === undefined || str === null || str === "") {
        return "npm";
    } else {

        // for the windows fam 
        const winRegEx = /\\/g;
        const winfiltr = str.replace(winRegEx, " ");

        // for the linux fam
        const linRegEx = /\//g;
        const filtr = winfiltr.replace(linRegEx, " ");

        // after making every slash into a space, split into array, get last element
        const bin = filtr.split(" ").pop();

        // poors man's match statement
        if (bin.includes("pnpm")) {
            return "pnpm";
        }
        
        if (bin.includes("yarn")) {
            return "yarn";
        }
        
        if (bin.includes("npm")){
            return "npm";
        }

        // default
        return "npm";
    }
}